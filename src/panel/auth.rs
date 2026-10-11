use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use axum::http::{header, HeaderMap};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use cookie::{Cookie, SameSite};
use rand::{rngs::OsRng, RngCore};
use redb::{Database, ReadableTable, TableDefinition};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    net::IpAddr,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const ADMIN: TableDefinition<&str, &str> = TableDefinition::new("panel_admin");
const PASSWORD_KEY: &str = "password_hash";
pub const COOKIE_NAME: &str = "panel_session";
const CLAIM_TTL: Duration = Duration::from_secs(15 * 60);
const IDLE_TTL: Duration = Duration::from_secs(30 * 60);
const ABSOLUTE_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const RATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_SESSIONS: usize = 128;
const MAX_RATE_PEERS: usize = 256;
const PASSWORD_MIN: usize = 12;
const PASSWORD_MAX: usize = 1024;
type TokenDigest = [u8; 32];
type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
pub enum AuthError {
    Required,
    InvalidCredentials,
    ClaimInvalid,
    ClaimUnavailable,
    CsrfInvalid,
    OriginInvalid,
    RateLimited,
    Busy,
    Storage(BoxError),
    Random,
    PasswordInvalid,
    SessionLimit,
    Internal,
}

impl AuthError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Required => "auth_required",
            Self::InvalidCredentials => "auth_invalid_credentials",
            Self::ClaimInvalid => "auth_claim_invalid",
            Self::ClaimUnavailable => "auth_claim_unavailable",
            Self::CsrfInvalid => "auth_csrf_invalid",
            Self::OriginInvalid => "auth_origin_invalid",
            Self::RateLimited => "auth_rate_limited",
            Self::Busy => "auth_busy",
            Self::Storage(_) => "auth_storage_failed",
            Self::Random => "auth_random_failed",
            Self::PasswordInvalid => "auth_password_invalid",
            Self::SessionLimit => "auth_session_limit",
            Self::Internal => "auth_internal",
        }
    }

    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Required => "panel.auth_required",
            Self::InvalidCredentials => "panel.auth_invalid_credentials",
            Self::ClaimInvalid => "panel.auth_claim_invalid",
            Self::ClaimUnavailable => "panel.auth_claim_unavailable",
            Self::CsrfInvalid => "panel.auth_csrf_invalid",
            Self::OriginInvalid => "panel.auth_origin_invalid",
            Self::RateLimited => "panel.auth_rate_limited",
            Self::Busy => "panel.auth_busy",
            Self::Storage(_) => "panel.auth_storage_failed",
            Self::Random => "panel.auth_random_failed",
            Self::PasswordInvalid => "panel.auth_password_invalid",
            Self::SessionLimit => "panel.auth_session_limit",
            Self::Internal => "panel.auth_internal",
        }
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message_key())
    }
}

impl Error for AuthError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

fn storage(error: impl Error + Send + Sync + 'static) -> AuthError {
    AuthError::Storage(Box::new(error))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionScope {
    Preauth,
    Setup,
    Admin,
}

#[derive(Clone, Serialize)]
pub struct SessionInfo {
    pub scope: SessionScope,
    pub csrf: String,
}

// Do not derive Debug: this value contains the only copy of the bearer credential.
pub struct SessionIssue {
    pub token: String,
    pub info: SessionInfo,
}

impl SessionIssue {
    pub fn cookie(&self, secure: bool) -> String {
        session_cookie(self.token.clone(), secure).to_string()
    }
}

fn session_cookie(token: String, secure: bool) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, token))
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(secure)
        .path("/")
        .build()
}

pub fn clear_cookie(secure: bool) -> String {
    let mut cookie = session_cookie(String::new(), secure);
    cookie.make_removal();
    cookie.to_string()
}

pub fn cookie_token(headers: &HeaderMap) -> Result<Option<String>, AuthError> {
    let mut token = None;
    for value in headers.get_all(header::COOKIE) {
        let value = value.to_str().map_err(|_| AuthError::Required)?;
        for cookie in Cookie::split_parse(value) {
            let cookie = cookie.map_err(|_| AuthError::Required)?;
            if cookie.name() == COOKIE_NAME {
                if token.is_some() || !valid_token(cookie.value()) {
                    return Err(AuthError::Required);
                }
                token = Some(cookie.value().to_owned());
            }
        }
    }
    Ok(token)
}

/// Public origins are exact; loopback aliases keep the same scheme and port.
#[derive(Clone)]
pub struct OriginPolicy {
    endpoints: Vec<(String, String)>,
    secure: bool,
}

impl OriginPolicy {
    pub fn new(origin: &str) -> Result<Self, AuthError> {
        let url = reqwest::Url::parse(origin).map_err(|_| AuthError::OriginInvalid)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(AuthError::OriginInvalid);
        }
        let endpoints = if matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        ) {
            let port = url
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default();
            ["localhost", "127.0.0.1", "[::1]"]
                .into_iter()
                .map(|host| {
                    let authority = format!("{host}{port}");
                    (format!("{}://{authority}", url.scheme()), authority)
                })
                .collect()
        } else {
            let origin = url.origin().ascii_serialization();
            let authority = origin
                .split_once("://")
                .ok_or(AuthError::OriginInvalid)?
                .1
                .to_owned();
            vec![(origin, authority)]
        };
        Ok(Self {
            endpoints,
            secure: url.scheme() == "https",
        })
    }

    pub fn secure(&self) -> bool {
        self.secure
    }

    pub fn check(&self, headers: &HeaderMap, mutation: bool) -> Result<(), AuthError> {
        let host = single_header(headers, header::HOST)?.ok_or(AuthError::OriginInvalid)?;
        let (expected_origin, _) = self
            .endpoints
            .iter()
            .find(|(_, authority)| host.eq_ignore_ascii_case(authority))
            .ok_or(AuthError::OriginInvalid)?;
        match single_header(headers, header::ORIGIN)? {
            Some(origin) if origin == expected_origin => Ok(()),
            None if !mutation => Ok(()),
            _ => Err(AuthError::OriginInvalid),
        }
    }
}

fn single_header(headers: &HeaderMap, name: header::HeaderName) -> Result<Option<&str>, AuthError> {
    let all = headers.get_all(name);
    let mut values = all.iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(AuthError::OriginInvalid);
    }
    first
        .map(|value| value.to_str().map_err(|_| AuthError::OriginInvalid))
        .transpose()
}

struct Session {
    scope: SessionScope,
    csrf: TokenDigest,
    created: Instant,
    touched: Instant,
}

impl Session {
    fn expired(&self, now: Instant) -> bool {
        let absolute = if self.scope == SessionScope::Admin {
            ABSOLUTE_TTL
        } else {
            CLAIM_TTL
        };
        now.duration_since(self.created) >= absolute || now.duration_since(self.touched) >= IDLE_TTL
    }
}

struct Claim {
    digest: TokenDigest,
    created: Instant,
}

struct RateBucket {
    started: Instant,
    attempts: u16,
}

struct AuthState {
    hash: Option<String>,
    generation: u64,
    claim: Option<Claim>,
    sessions: HashMap<TokenDigest, Session>,
    peers: HashMap<IpAddr, RateBucket>,
    global: RateBucket,
}

impl AuthState {
    fn prune(&mut self, now: Instant) {
        self.sessions.retain(|_, session| !session.expired(now));
        self.peers
            .retain(|_, bucket| now.duration_since(bucket.started) < RATE_WINDOW);
    }

    fn rate_limit(&mut self, peer: IpAddr, now: Instant) -> Result<(), AuthError> {
        self.prune(now);
        if now.duration_since(self.global.started) >= RATE_WINDOW {
            self.global = RateBucket {
                started: now,
                attempts: 0,
            };
        }
        if self.global.attempts >= 60 {
            return Err(AuthError::RateLimited);
        }
        self.global.attempts += 1;
        if !self.peers.contains_key(&peer) && self.peers.len() >= MAX_RATE_PEERS {
            return Err(AuthError::RateLimited);
        }
        let bucket = self.peers.entry(peer).or_insert(RateBucket {
            started: now,
            attempts: 0,
        });
        if bucket.attempts >= 6 {
            return Err(AuthError::RateLimited);
        }
        bucket.attempts += 1;
        Ok(())
    }

    fn authorize(
        &mut self,
        token: &str,
        csrf: Option<&str>,
        scope: SessionScope,
    ) -> Result<(), AuthError> {
        if !valid_token(token) {
            return Err(AuthError::Required);
        }
        let now = Instant::now();
        self.prune(now);
        let session = self
            .sessions
            .get_mut(&digest(token.as_bytes()))
            .ok_or(AuthError::Required)?;
        if session.scope != scope {
            return Err(AuthError::Required);
        }
        if let Some(csrf) = csrf {
            if !valid_token(csrf) || !constant_eq(&session.csrf, &digest(csrf.as_bytes())) {
                return Err(AuthError::CsrfInvalid);
            }
        }
        session.touched = now;
        Ok(())
    }

    fn insert(&mut self, issue: &SessionIssue, now: Instant) {
        self.sessions.insert(
            digest(issue.token.as_bytes()),
            Session {
                scope: issue.info.scope,
                csrf: digest(issue.info.csrf.as_bytes()),
                created: now,
                touched: now,
            },
        );
    }
}

#[derive(Clone)]
enum Store {
    Shared(&'static Database),
    #[cfg(test)]
    Test(Arc<Database>),
}

impl Store {
    fn db(&self) -> &Database {
        match self {
            Self::Shared(db) => db,
            #[cfg(test)]
            Self::Test(db) => db,
        }
    }
}

struct Inner {
    store: Store,
    state: Mutex<AuthState>,
    blocking: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct AuthService {
    inner: Arc<Inner>,
}

impl AuthService {
    pub async fn new() -> Result<Self, AuthError> {
        Self::from_store(Store::Shared(crate::memory::db())).await
    }

    #[cfg(test)]
    pub(super) async fn isolated(path: &std::path::Path) -> Result<Self, AuthError> {
        Self::from_store(Store::Test(Arc::new(
            Database::create(path).map_err(storage)?,
        )))
        .await
    }

    async fn from_store(store: Store) -> Result<Self, AuthError> {
        let load_store = store.clone();
        let hash = tokio::task::spawn_blocking(move || load_hash(load_store.db()))
            .await
            .map_err(|_| AuthError::Internal)??;
        let now = Instant::now();
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                state: Mutex::new(AuthState {
                    hash,
                    generation: 0,
                    claim: None,
                    sessions: HashMap::new(),
                    peers: HashMap::new(),
                    global: RateBucket {
                        started: now,
                        attempts: 0,
                    },
                }),
                blocking: Arc::new(Semaphore::new(2)),
            }),
        })
    }

    fn state(&self) -> Result<MutexGuard<'_, AuthState>, AuthError> {
        self.inner.state.lock().map_err(|_| AuthError::Internal)
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, AuthError> + Send + 'static,
    ) -> Result<T, AuthError> {
        let permit = self
            .inner
            .blocking
            .clone()
            .try_acquire_owned()
            .map_err(|_| AuthError::Busy)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|_| AuthError::Internal)?
    }

    pub fn has_admin(&self) -> Result<bool, AuthError> {
        Ok(self.state()?.hash.is_some())
    }

    /// Return directly to the console only, never to a public HTTP response or logger.
    pub fn renew_claim(&self) -> Result<Option<String>, AuthError> {
        let mut state = self.state()?;
        if state.hash.is_some() {
            return Ok(None);
        }
        let token = random_token()?;
        state
            .sessions
            .retain(|_, session| session.scope != SessionScope::Setup);
        state.claim = Some(Claim {
            digest: digest(token.as_bytes()),
            created: Instant::now(),
        });
        Ok(Some(token))
    }

    /// Public login/bootstrap sessions are rate-limited before allocating credentials.
    pub fn preauth(&self, peer: IpAddr) -> Result<SessionIssue, AuthError> {
        let mut state = self.state()?;
        state.rate_limit(peer, Instant::now())?;
        if state.sessions.len() >= MAX_SESSIONS {
            return Err(AuthError::SessionLimit);
        }
        let issue = new_issue(SessionScope::Preauth)?;
        state.insert(&issue, Instant::now());
        Ok(issue)
    }

    pub fn claim(
        &self,
        token: &str,
        preauth: &str,
        csrf: &str,
        peer: IpAddr,
    ) -> Result<SessionIssue, AuthError> {
        let mut state = self.state()?;
        let now = Instant::now();
        state.rate_limit(peer, now)?;
        state.authorize(preauth, Some(csrf), SessionScope::Preauth)?;
        if state.hash.is_some() {
            return Err(AuthError::ClaimUnavailable);
        }
        let claim = state.claim.as_ref().ok_or(AuthError::ClaimInvalid)?;
        if !valid_token(token)
            || now.duration_since(claim.created) >= CLAIM_TTL
            || !constant_eq(&claim.digest, &digest(token.as_bytes()))
        {
            return Err(AuthError::ClaimInvalid);
        }
        let issue = new_issue(SessionScope::Setup)?;
        state.claim = None;
        state.sessions.remove(&digest(preauth.as_bytes()));
        state.insert(&issue, now);
        Ok(issue)
    }

    pub fn session(&self, token: &str) -> Result<SessionInfo, AuthError> {
        if !valid_token(token) {
            return Err(AuthError::Required);
        }
        let mut state = self.state()?;
        let now = Instant::now();
        state.prune(now);
        let session = state
            .sessions
            .get_mut(&digest(token.as_bytes()))
            .ok_or(AuthError::Required)?;
        session.touched = now;
        Ok(SessionInfo {
            scope: session.scope,
            csrf: csrf_token(token),
        })
    }

    pub fn require_admin(&self, token: &str) -> Result<(), AuthError> {
        self.state()?.authorize(token, None, SessionScope::Admin)
    }

    /// Automatic refreshes and established streams must not keep an idle owner signed in.
    pub fn require_admin_passive(&self, token: &str) -> Result<(), AuthError> {
        if !valid_token(token) {
            return Err(AuthError::Required);
        }
        let mut state = self.state()?;
        state.prune(Instant::now());
        let session = state
            .sessions
            .get(&digest(token.as_bytes()))
            .ok_or(AuthError::Required)?;
        if session.scope != SessionScope::Admin {
            return Err(AuthError::Required);
        }
        Ok(())
    }

    pub fn require_setup(&self, token: &str) -> Result<(), AuthError> {
        self.state()?.authorize(token, None, SessionScope::Setup)
    }

    pub fn mutation(&self, token: &str, csrf: &str, scope: SessionScope) -> Result<(), AuthError> {
        self.state()?.authorize(token, Some(csrf), scope)
    }

    /// Apply to credential tests too, before network calls or provider billable work.
    pub fn rate_limit(&self, peer: IpAddr) -> Result<(), AuthError> {
        self.state()?.rate_limit(peer, Instant::now())
    }

    pub async fn create_admin(
        &self,
        password: String,
        setup: &str,
        csrf: &str,
        peer: IpAddr,
    ) -> Result<SessionIssue, AuthError> {
        {
            let mut state = self.state()?;
            state.rate_limit(peer, Instant::now())?;
            state.authorize(setup, Some(csrf), SessionScope::Setup)?;
            if state.hash.is_some() {
                return Err(AuthError::ClaimUnavailable);
            }
        }
        validate_password(&password)?;
        let hash = self.blocking(move || hash_password(&password)).await?;
        let issue = new_issue(SessionScope::Admin)?;
        let inner = self.inner.clone();
        let setup = setup.to_owned();
        let csrf = csrf.to_owned();
        self.blocking(move || {
            let mut state = inner.state.lock().map_err(|_| AuthError::Internal)?;
            state.authorize(&setup, Some(&csrf), SessionScope::Setup)?;
            if state.hash.is_some() {
                return Err(AuthError::ClaimUnavailable);
            }
            replace_hash(inner.store.db(), None, &hash)?;
            state.hash = Some(hash);
            state.generation = state.generation.wrapping_add(1);
            state.claim = None;
            state.sessions.clear();
            state.insert(&issue, Instant::now());
            Ok(issue)
        })
        .await
    }

    pub async fn login(
        &self,
        password: String,
        preauth: &str,
        csrf: &str,
        peer: IpAddr,
    ) -> Result<SessionIssue, AuthError> {
        let (hash, generation) = {
            let mut state = self.state()?;
            state.rate_limit(peer, Instant::now())?;
            state.authorize(preauth, Some(csrf), SessionScope::Preauth)?;
            (
                state.hash.clone().ok_or(AuthError::InvalidCredentials)?,
                state.generation,
            )
        };
        if password.len() > PASSWORD_MAX {
            return Err(AuthError::InvalidCredentials);
        }
        self.blocking(move || verify_password(&password, &hash))
            .await?;
        let issue = new_issue(SessionScope::Admin)?;
        let mut state = self.state()?;
        state.authorize(preauth, Some(csrf), SessionScope::Preauth)?;
        if state.generation != generation {
            return Err(AuthError::InvalidCredentials);
        }
        state.sessions.remove(&digest(preauth.as_bytes()));
        state.insert(&issue, Instant::now());
        Ok(issue)
    }

    pub fn logout(&self, token: &str, csrf: &str) -> Result<(), AuthError> {
        let mut state = self.state()?;
        let scope = state
            .sessions
            .get(&digest(token.as_bytes()))
            .ok_or(AuthError::Required)?
            .scope;
        state.authorize(token, Some(csrf), scope)?;
        state.sessions.remove(&digest(token.as_bytes()));
        Ok(())
    }

    pub async fn change_password(
        &self,
        current: String,
        password: String,
        token: &str,
        csrf: &str,
        peer: IpAddr,
    ) -> Result<SessionIssue, AuthError> {
        let (old_hash, generation) = {
            let mut state = self.state()?;
            state.rate_limit(peer, Instant::now())?;
            state.authorize(token, Some(csrf), SessionScope::Admin)?;
            (
                state.hash.clone().ok_or(AuthError::Required)?,
                state.generation,
            )
        };
        validate_password(&password)?;
        if current.len() > PASSWORD_MAX {
            return Err(AuthError::InvalidCredentials);
        }
        let expected = old_hash.clone();
        let hash = self
            .blocking(move || {
                verify_password(&current, &old_hash)?;
                hash_password(&password)
            })
            .await?;
        let issue = new_issue(SessionScope::Admin)?;
        let inner = self.inner.clone();
        let token = token.to_owned();
        let csrf = csrf.to_owned();
        self.blocking(move || {
            let mut state = inner.state.lock().map_err(|_| AuthError::Internal)?;
            state.authorize(&token, Some(&csrf), SessionScope::Admin)?;
            if state.generation != generation {
                return Err(AuthError::InvalidCredentials);
            }
            replace_hash(inner.store.db(), Some(&expected), &hash)?;
            state.hash = Some(hash);
            state.generation = state.generation.wrapping_add(1);
            state.sessions.clear();
            state.insert(&issue, Instant::now());
            Ok(issue)
        })
        .await
    }

    /// CLI-only replacement preserves ownership; print the returned password once.
    /// redb refuses a competing process opening the live database, so operators must stop it.
    pub async fn reset_password(&self) -> Result<String, AuthError> {
        let password = random_token()?;
        let hash_password_input = password.clone();
        let hash = self
            .blocking(move || hash_password(&hash_password_input))
            .await?;
        let inner = self.inner.clone();
        self.blocking(move || {
            let mut state = inner.state.lock().map_err(|_| AuthError::Internal)?;
            let expected = state.hash.as_deref().ok_or(AuthError::ClaimUnavailable)?;
            replace_hash(inner.store.db(), Some(expected), &hash)?;
            state.hash = Some(hash);
            state.generation = state.generation.wrapping_add(1);
            state.claim = None;
            state.sessions.clear();
            Ok(())
        })
        .await?;
        Ok(password)
    }
}

fn load_hash(db: &Database) -> Result<Option<String>, AuthError> {
    let txn = db.begin_write().map_err(storage)?;
    let hash = {
        let table = txn.open_table(ADMIN).map_err(storage)?;
        let hash = table
            .get(PASSWORD_KEY)
            .map_err(storage)?
            .map(|value| value.value().to_owned());
        if let Some(hash) = &hash {
            checked_hash(hash)?;
        }
        hash
    };
    txn.commit().map_err(storage)?;
    Ok(hash)
}

fn replace_hash(db: &Database, expected: Option<&str>, hash: &str) -> Result<(), AuthError> {
    let txn = db.begin_write().map_err(storage)?;
    {
        let mut table = txn.open_table(ADMIN).map_err(storage)?;
        let current = table
            .get(PASSWORD_KEY)
            .map_err(storage)?
            .map(|value| value.value().to_owned());
        if current.as_deref() != expected {
            return Err(AuthError::InvalidCredentials);
        }
        table.insert(PASSWORD_KEY, hash).map_err(storage)?;
    }
    txn.commit().map_err(storage)?;
    Ok(())
}

fn argon2() -> Result<Argon2<'static>, AuthError> {
    let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|_| AuthError::Internal)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

fn validate_password(password: &str) -> Result<(), AuthError> {
    if !(PASSWORD_MIN..=PASSWORD_MAX).contains(&password.len()) {
        return Err(AuthError::PasswordInvalid);
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, AuthError> {
    let mut salt = [0u8; 16];
    OsRng
        .try_fill_bytes(&mut salt)
        .map_err(|_| AuthError::Random)?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| AuthError::Internal)?;
    argon2()?
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::Internal)
}

// Never let corrupted PHC parameters allocate unbounded memory during verification.
fn checked_hash(hash: &str) -> Result<PasswordHash<'_>, AuthError> {
    let invalid = || {
        storage(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid admin password hash",
        ))
    };
    if hash.len() > 256 {
        return Err(invalid());
    }
    let parsed = PasswordHash::new(hash).map_err(|_| invalid())?;
    if parsed.algorithm.as_str() != "argon2id"
        || parsed.version != Some(19)
        || parsed.params.get_decimal("m") != Some(19 * 1024)
        || parsed.params.get_decimal("t") != Some(2)
        || parsed.params.get_decimal("p") != Some(1)
        || parsed.params.iter().count() != 3
        || parsed.salt.is_none()
        || parsed.hash.as_ref().is_none_or(|value| value.len() != 32)
    {
        return Err(invalid());
    }
    Ok(parsed)
}

fn verify_password(password: &str, hash: &str) -> Result<(), AuthError> {
    let hash = checked_hash(hash)?;
    argon2()?
        .verify_password(password.as_bytes(), &hash)
        .map_err(|_| AuthError::InvalidCredentials)
}

fn random_token() -> Result<String, AuthError> {
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| AuthError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn valid_token(token: &str) -> bool {
    if token.len() != 43 {
        return false;
    }
    let mut decoded = [0u8; 32];
    URL_SAFE_NO_PAD
        .decode_slice(token, &mut decoded)
        .is_ok_and(|size| size == 32)
}

fn digest(bytes: &[u8]) -> TokenDigest {
    Sha256::digest(bytes).into()
}

fn csrf_token(token: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"panel-csrf-v1:");
    hash.update(token.as_bytes());
    URL_SAFE_NO_PAD.encode(hash.finalize())
}

fn new_issue(scope: SessionScope) -> Result<SessionIssue, AuthError> {
    let token = random_token()?;
    let csrf = csrf_token(&token);
    Ok(SessionIssue {
        token,
        info: SessionInfo { scope, csrf },
    })
}

fn constant_eq(left: &TokenDigest, right: &TokenDigest) -> bool {
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn peer() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }

    async fn service() -> (AuthService, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let database = Arc::new(Database::create(directory.path().join("auth.redb")).unwrap());
        let service = AuthService::from_store(Store::Test(database))
            .await
            .unwrap();
        (service, directory)
    }

    fn claim_setup(service: &AuthService) -> SessionIssue {
        let claim = service.renew_claim().unwrap().unwrap();
        let preauth = service.preauth(peer()).unwrap();
        service
            .claim(&claim, &preauth.token, &preauth.info.csrf, peer())
            .unwrap()
    }

    async fn admin(service: &AuthService) -> SessionIssue {
        let setup = claim_setup(service);
        service
            .create_admin(
                "a sufficiently long password".into(),
                &setup.token,
                &setup.info.csrf,
                peer(),
            )
            .await
            .unwrap()
    }

    fn clear_rate(service: &AuthService) {
        let mut state = service.state().unwrap();
        state.peers.clear();
        state.global.attempts = 0;
    }

    #[tokio::test]
    async fn claim_is_once_and_restricted_and_creates_no_admin() {
        let (service, _directory) = service().await;
        let claim = service.renew_claim().unwrap().unwrap();
        let preauth = service.preauth(peer()).unwrap();
        let setup = service
            .claim(&claim, &preauth.token, &preauth.info.csrf, peer())
            .unwrap();
        assert_eq!(setup.info.scope, SessionScope::Setup);
        assert!(!service.has_admin().unwrap());
        assert!(service.require_admin(&setup.token).is_err());
        assert!(service.session(&preauth.token).is_err());
        let retry = service.preauth(peer()).unwrap();
        assert!(matches!(
            service.claim(&claim, &retry.token, &retry.info.csrf, peer()),
            Err(AuthError::ClaimInvalid),
        ));
    }

    #[tokio::test]
    async fn claims_expire_and_renewal_revokes_setup() {
        let (service, _directory) = service().await;
        let claim = service.renew_claim().unwrap().unwrap();
        let preauth = service.preauth(peer()).unwrap();
        service.state().unwrap().claim.as_mut().unwrap().created = Instant::now() - CLAIM_TTL;
        assert!(matches!(
            service.claim(&claim, &preauth.token, &preauth.info.csrf, peer()),
            Err(AuthError::ClaimInvalid),
        ));
        clear_rate(&service);
        let setup = claim_setup(&service);
        service.renew_claim().unwrap();
        assert!(service.session(&setup.token).is_err());
    }

    #[tokio::test]
    async fn csrf_required_for_claim_login_and_mutations() {
        let (service, _directory) = service().await;
        let claim = service.renew_claim().unwrap().unwrap();
        let preauth = service.preauth(peer()).unwrap();
        assert!(matches!(
            service.claim(&claim, &preauth.token, &random_token().unwrap(), peer()),
            Err(AuthError::CsrfInvalid),
        ));
        let setup = service
            .claim(&claim, &preauth.token, &preauth.info.csrf, peer())
            .unwrap();
        assert!(matches!(
            service
                .create_admin(
                    "a sufficiently long password".into(),
                    &setup.token,
                    "",
                    peer(),
                )
                .await,
            Err(AuthError::CsrfInvalid),
        ));
        clear_rate(&service);
        let admin = service
            .create_admin(
                "a sufficiently long password".into(),
                &setup.token,
                &setup.info.csrf,
                peer(),
            )
            .await
            .unwrap();
        assert!(service
            .mutation(&admin.token, "", SessionScope::Admin)
            .is_err());
        assert!(service.logout(&admin.token, "").is_err());
        clear_rate(&service);
        let preauth = service.preauth(peer()).unwrap();
        assert!(matches!(
            service
                .login(
                    "a sufficiently long password".into(),
                    &preauth.token,
                    "",
                    peer(),
                )
                .await,
            Err(AuthError::CsrfInvalid),
        ));
    }

    #[tokio::test]
    async fn ownership_persists_and_password_verification_rotates_session() {
        let (service, _directory) = service().await;
        let admin = admin(&service).await;
        assert!(service.has_admin().unwrap());
        assert!(service.renew_claim().unwrap().is_none());
        let hash = load_hash(service.inner.store.db()).unwrap().unwrap();
        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(!hash.contains("sufficiently"));
        let reloaded = AuthService::from_store(service.inner.store.clone())
            .await
            .unwrap();
        assert!(reloaded.has_admin().unwrap());
        assert!(reloaded.session(&admin.token).is_err());
        let preauth = reloaded.preauth(peer()).unwrap();
        assert!(matches!(
            reloaded
                .login("wrong".into(), &preauth.token, &preauth.info.csrf, peer())
                .await,
            Err(AuthError::InvalidCredentials),
        ));
        let logged_in = reloaded
            .login(
                "a sufficiently long password".into(),
                &preauth.token,
                &preauth.info.csrf,
                peer(),
            )
            .await
            .unwrap();
        assert_ne!(preauth.token, logged_in.token);
        assert!(reloaded.session(&preauth.token).is_err());
        assert!(reloaded.require_admin(&logged_in.token).is_ok());
        reloaded
            .logout(&logged_in.token, &logged_in.info.csrf)
            .unwrap();
        assert!(reloaded.session(&logged_in.token).is_err());
    }

    #[tokio::test]
    async fn password_change_revokes_all_and_reset_retains_ownership() {
        let (service, _directory) = service().await;
        let initial = admin(&service).await;
        clear_rate(&service);
        let preauth = service.preauth(peer()).unwrap();
        let second = service
            .login(
                "a sufficiently long password".into(),
                &preauth.token,
                &preauth.info.csrf,
                peer(),
            )
            .await
            .unwrap();
        let changed = service
            .change_password(
                "a sufficiently long password".into(),
                "the replacement password".into(),
                &initial.token,
                &initial.info.csrf,
                peer(),
            )
            .await
            .unwrap();
        assert_ne!(initial.token, changed.token);
        assert!(service.session(&initial.token).is_err());
        assert!(service.session(&second.token).is_err());
        let hash = load_hash(service.inner.store.db()).unwrap().unwrap();
        assert!(verify_password("a sufficiently long password", &hash).is_err());
        assert!(verify_password("the replacement password", &hash).is_ok());
        let reset = service.reset_password().await.unwrap();
        assert!(service.has_admin().unwrap());
        assert!(service.renew_claim().unwrap().is_none());
        assert!(service.session(&changed.token).is_err());
        let hash = load_hash(service.inner.store.db()).unwrap().unwrap();
        assert!(verify_password("the replacement password", &hash).is_err());
        assert!(verify_password(&reset, &hash).is_ok());
    }

    #[tokio::test]
    async fn session_idle_absolute_setup_expiry_and_capacity_are_bounded() {
        let (service, _directory) = service().await;
        let admin = admin(&service).await;
        {
            let mut state = service.state().unwrap();
            state
                .sessions
                .get_mut(&digest(admin.token.as_bytes()))
                .unwrap()
                .touched = Instant::now() - IDLE_TTL;
        }
        assert!(service.session(&admin.token).is_err());
        let absolute = new_issue(SessionScope::Admin).unwrap();
        {
            let mut state = service.state().unwrap();
            state.insert(&absolute, Instant::now());
            state
                .sessions
                .get_mut(&digest(absolute.token.as_bytes()))
                .unwrap()
                .created = Instant::now() - ABSOLUTE_TTL;
        }
        assert!(service.session(&absolute.token).is_err());
        let setup = new_issue(SessionScope::Setup).unwrap();
        {
            let mut state = service.state().unwrap();
            state.insert(&setup, Instant::now());
            state
                .sessions
                .get_mut(&digest(setup.token.as_bytes()))
                .unwrap()
                .created = Instant::now() - CLAIM_TTL;
        }
        assert!(service.session(&setup.token).is_err());
        {
            let mut state = service.state().unwrap();
            for _ in 0..MAX_SESSIONS {
                let issue = new_issue(SessionScope::Preauth).unwrap();
                state.insert(&issue, Instant::now());
            }
        }
        clear_rate(&service);
        assert!(matches!(
            service.preauth(peer()),
            Err(AuthError::SessionLimit)
        ));
    }

    #[tokio::test]
    async fn passive_authorization_does_not_extend_idle_expiry() {
        let (service, _directory) = service().await;
        let admin = admin(&service).await;
        let touched = Instant::now() - Duration::from_secs(10);
        service
            .state()
            .unwrap()
            .sessions
            .get_mut(&digest(admin.token.as_bytes()))
            .unwrap()
            .touched = touched;
        service.require_admin_passive(&admin.token).unwrap();
        assert_eq!(
            service.state().unwrap().sessions[&digest(admin.token.as_bytes())].touched,
            touched
        );
        service
            .state()
            .unwrap()
            .sessions
            .get_mut(&digest(admin.token.as_bytes()))
            .unwrap()
            .touched = Instant::now() - IDLE_TTL;
        assert!(matches!(
            service.require_admin_passive(&admin.token),
            Err(AuthError::Required)
        ));
    }

    #[tokio::test]
    async fn limiter_and_hash_semaphore_fail_before_costly_work() {
        let (service, _directory) = service().await;
        for _ in 0..6 {
            service.rate_limit(peer()).unwrap();
        }
        assert!(matches!(
            service.rate_limit(peer()),
            Err(AuthError::RateLimited)
        ));
        service
            .state()
            .unwrap()
            .peers
            .get_mut(&peer())
            .unwrap()
            .started = Instant::now() - RATE_WINDOW;
        service.rate_limit(peer()).unwrap();
        let permits = service
            .inner
            .blocking
            .clone()
            .acquire_many_owned(2)
            .await
            .unwrap();
        assert!(matches!(
            service.blocking(|| Ok(())).await,
            Err(AuthError::Busy)
        ));
        drop(permits);
    }

    #[tokio::test]
    async fn corrupt_hash_fails_closed_and_failed_creation_keeps_setup() {
        let (service, _directory) = service().await;
        let setup = claim_setup(&service);
        assert!(matches!(
            service
                .create_admin("short".into(), &setup.token, &setup.info.csrf, peer())
                .await,
            Err(AuthError::PasswordInvalid),
        ));
        assert!(service.require_setup(&setup.token).is_ok());
        replace_hash(service.inner.store.db(), None, "not a password hash").unwrap();
        assert!(matches!(
            AuthService::from_store(service.inner.store.clone()).await,
            Err(AuthError::Storage(_)),
        ));
        clear_rate(&service);
        assert!(matches!(
            service
                .create_admin(
                    "a sufficiently long password".into(),
                    &setup.token,
                    &setup.info.csrf,
                    peer(),
                )
                .await,
            Err(AuthError::InvalidCredentials),
        ));
        assert!(!service.has_admin().unwrap());
        assert!(service.require_setup(&setup.token).is_ok());
    }

    #[test]
    fn origins_hosts_and_forwarded_values_are_strict() {
        let policy = OriginPolicy::new("http://localhost:8787").unwrap();
        assert!(!policy.secure());
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:8787"));
        assert!(policy.check(&headers, false).is_ok());
        assert!(policy.check(&headers, true).is_err());
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://localhost:8787"),
        );
        assert!(policy.check(&headers, true).is_ok());
        headers.insert(header::ORIGIN, HeaderValue::from_static("null"));
        assert!(policy.check(&headers, true).is_err());
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://localhost:8787"),
        );
        headers.insert(header::HOST, HeaderValue::from_static("evil.example:8787"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("localhost:8787"),
        );
        assert!(policy.check(&headers, true).is_err());
        headers.insert(header::HOST, HeaderValue::from_static("localhost"));
        assert!(policy.check(&headers, true).is_err());
        headers.insert(header::HOST, HeaderValue::from_static("localhost:8787"));
        headers.append(
            header::ORIGIN,
            HeaderValue::from_static("http://localhost:8787"),
        );
        assert!(policy.check(&headers, true).is_err());
        assert!(OriginPolicy::new("https://panel.example").unwrap().secure());
        for url in [
            "ftp://panel.example",
            "https://user@panel.example",
            "https://panel.example/path",
            "https://panel.example?query=1",
            "https://panel.example#fragment",
        ] {
            assert!(OriginPolicy::new(url).is_err());
        }
    }

    #[test]
    fn cookies_are_http_only_strict_host_only_and_secure_for_https() {
        let issue = new_issue(SessionScope::Admin).unwrap();
        let local = Cookie::parse(issue.cookie(false)).unwrap();
        assert_eq!(local.http_only(), Some(true));
        assert_eq!(local.same_site(), Some(SameSite::Strict));
        assert_eq!(local.path(), Some("/"));
        assert_eq!(local.domain(), None);
        assert!(!local.secure().unwrap_or(false));
        let tls = Cookie::parse(issue.cookie(true)).unwrap();
        assert_eq!(tls.secure(), Some(true));
        let removed = Cookie::parse(clear_cookie(true)).unwrap();
        assert_eq!(removed.max_age().unwrap().whole_seconds(), 0);
        assert_eq!(removed.path(), Some("/"));
        assert_eq!(removed.http_only(), Some(true));
        assert_eq!(removed.secure(), Some(true));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("other=value; {COOKIE_NAME}={}", issue.token)).unwrap(),
        );
        assert_eq!(
            cookie_token(&headers).unwrap().as_deref(),
            Some(issue.token.as_str())
        );
        headers.append(
            header::COOKIE,
            HeaderValue::from_str(&format!("{COOKIE_NAME}={}", issue.token)).unwrap(),
        );
        assert!(cookie_token(&headers).is_err());
    }

    #[test]
    fn password_bounds_and_hash_parameter_corruption_are_rejected() {
        assert!(validate_password("short").is_err());
        assert!(validate_password(&"x".repeat(PASSWORD_MAX + 1)).is_err());
        assert!(validate_password("correct horse battery staple").is_ok());
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash).is_ok());
        assert!(verify_password("incorrect password", &hash).is_err());
        assert!(checked_hash(&hash.replace("m=19456", "m=4294967295")).is_err());
        assert!(checked_hash(&hash.replace("argon2id", "argon2i")).is_err());
    }

    #[test]
    fn loopback_aliases_keep_host_origin_scheme_and_port_bound() {
        for configured in [
            "http://localhost:8787",
            "http://127.0.0.1:8787",
            "http://[::1]:8787",
        ] {
            let policy = OriginPolicy::new(configured).unwrap();
            for host in ["localhost:8787", "127.0.0.1:8787", "[::1]:8787"] {
                let mut headers = HeaderMap::new();
                headers.insert(header::HOST, HeaderValue::from_static(host));
                assert!(policy.check(&headers, false).is_ok());
                assert!(policy.check(&headers, true).is_err());
                headers.insert(
                    header::ORIGIN,
                    HeaderValue::from_str(&format!("http://{host}")).unwrap(),
                );
                assert!(policy.check(&headers, true).is_ok());
                headers.insert(
                    header::ORIGIN,
                    HeaderValue::from_str(&format!("https://{host}")).unwrap(),
                );
                assert!(policy.check(&headers, true).is_err());
            }
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8787"));
            headers.insert(
                header::ORIGIN,
                HeaderValue::from_static("http://localhost:8787"),
            );
            assert!(policy.check(&headers, true).is_err());
            headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8788"));
            assert!(policy.check(&headers, false).is_err());
        }
        let policy = OriginPolicy::new("https://panel.example").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8787"));
        assert!(policy.check(&headers, false).is_err());
        headers.insert(header::HOST, HeaderValue::from_static("panel.example"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://panel.example"),
        );
        assert!(policy.check(&headers, true).is_ok());
    }
}
