// One-time (safe to re-run) migrator: reads an old-style `durum/` markdown tree — from
// before the redb migration (see memory.rs's module doc) — and imports every record into a
// redb database, preserving each file's real modification time so `memory::files`'s
// "most recently changed first" ordering doesn't reset to "migration day" for everyone.
// `durum/arsiv/` is never touched: it stays real files, see memory.rs.
//
//   cargo run -- migrate-durum [--from <dir>] [--to <redb-path>] [--dry-run] [--force]
//
// Never deletes or moves the source .md files — non-destructive, so the operator can start
// the bot against the new database, sanity-check `/durum`/`/zihin`, and only then remove
// the old tree by hand.

use super::*;
use std::fs;
use std::path::{Path, PathBuf};

/// One migrated record: its database key, content, and source file's real mtime (unix
/// seconds, `0` if it couldn't be read). Produced by `collect` below.
struct Record {
    key: String,
    content: String,
    modified: u64,
}

pub fn run(args: &[String]) -> Result<(), BotError> {
    let mut from = PathBuf::from(STATE_DIR);
    let mut to = PathBuf::from(STATE_DIR).join("hafiza.redb");
    let mut dry_run = false;
    let mut force = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--from" => {
                i += 1;
                from = PathBuf::from(args.get(i).ok_or("--from bir değer ister")?);
            }
            "--to" => {
                i += 1;
                to = PathBuf::from(args.get(i).ok_or("--to bir değer ister")?);
            }
            "--dry-run" => dry_run = true,
            "--force" => force = true,
            other => return Err(format!("bilinmeyen argüman: {other}").into()),
        }
        i += 1;
    }

    let records = collect(&from, &from, "arsiv");
    let (kisi, konu, olay, tekil) = counts(&records);
    println!(
        "{} altında bulundu: {kisi} kişi, {konu} konu, {olay} ay (olaylar), {tekil} tekil kayıt — toplam {}",
        from.display(),
        records.len()
    );

    if records.is_empty() {
        println!("taşınacak .md dosyası yok, çıkılıyor");
        return Ok(());
    }
    if dry_run {
        println!("--dry-run: hiçbir şey yazılmadı");
        return Ok(());
    }

    memory::init(&to);
    let existing = memory::record_count();
    if existing > 0 && !force {
        return Err(format!(
            "{} zaten {existing} kayıt içeriyor; üzerine yazmak için --force ekle",
            to.display()
        )
        .into());
    }

    for r in &records {
        memory::write_with_mtime(&r.key, &r.content, r.modified);
    }

    println!(
        "tamamlandı: {} kayıt {} içine yazıldı",
        records.len(),
        to.display()
    );
    println!("orijinal .md dosyaları dokunulmadan kaldı — /durum ve /zihin ile doğruladıktan sonra elle silebilirsin");
    Ok(())
}

/// Recursively collects markdown records with `/`-joined keys relative to `base`.
/// Skips `skip_top_level` only directly under `base`, so same-named person/topic
/// files are still included.
fn collect(dir: &Path, base: &Path, skip_top_level: &str) -> Vec<Record> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if dir == base && path.file_name().and_then(|n| n.to_str()) == Some(skip_top_level) {
                continue; // durum/arsiv/: never migrated, see memory.rs's module doc
            }
            out.extend(collect(&path, base, skip_top_level));
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(base) else {
            continue;
        };
        let key = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let content = fs::read_to_string(&path).unwrap_or_default();
        let modified = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push(Record {
            key,
            content,
            modified,
        });
    }
    out
}

fn counts(records: &[Record]) -> (usize, usize, usize, usize) {
    let mut kisi = 0;
    let mut konu = 0;
    let mut olay = 0;
    let mut tekil = 0;
    for r in records {
        if r.key.starts_with("kisiler/") {
            kisi += 1;
        } else if r.key.starts_with("konular/") {
            konu += 1;
        } else if r.key.starts_with("olaylar/") {
            olay += 1;
        } else {
            tekil += 1;
        }
    }
    (kisi, konu, olay, tekil)
}

#[cfg(test)]
mod test {
    use super::*;

    /// Verifies `collect` finds `.md` files recursively, computes `/`-joined keys, skips
    /// only the top-level `arsiv/` (not a same-named file elsewhere), and ignores non-`.md`
    /// files — against a small fixture tree in the documented on-disk format.
    #[test]
    fn collect_walks_and_excludes_top_level_arsiv() {
        let root = std::env::temp_dir().join(format!(
            "discord-bot-test-migrate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("kisiler")).unwrap();
        fs::create_dir_all(root.join("konular")).unwrap();
        fs::create_dir_all(root.join("arsiv/kisiler")).unwrap();
        fs::write(root.join("profil.md"), "grup profili").unwrap();
        fs::write(root.join("model.md"), "bir-model").unwrap();
        fs::write(
            root.join("kisiler/1.md"),
            "# Ali\nid: 1\npuan: +2\netiket: \nnot: \n\n## Bildiklerin\n\n## Son olaylar\n",
        )
        .unwrap();
        fs::write(root.join("konular/rust.md"), "# rust\netiket: \n\n- not\n").unwrap();
        fs::write(root.join("arsiv/kisiler/1.md"), "eski hal").unwrap();
        fs::write(root.join("not-markdown.txt"), "atlanmalı").unwrap();

        let records = collect(&root, &root, "arsiv");
        let keys: Vec<&str> = records.iter().map(|r| r.key.as_str()).collect();

        assert!(keys.contains(&"profil.md"));
        assert!(keys.contains(&"model.md"));
        assert!(keys.contains(&"kisiler/1.md"));
        assert!(keys.contains(&"konular/rust.md"));
        assert!(!keys.iter().any(|k| k.starts_with("arsiv/")));
        assert!(!keys.iter().any(|k| k.contains("not-markdown")));

        let (kisi, konu, olay, tekil) = counts(&records);
        assert_eq!(kisi, 1);
        assert_eq!(konu, 1);
        assert_eq!(olay, 0);
        assert_eq!(tekil, 2); // profil.md + model.md

        let _ = fs::remove_dir_all(&root);
    }
}
