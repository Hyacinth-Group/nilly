# Launch drafts

These are drafts for the author to adapt and post manually. Nothing here has been posted.
Use the roommate story for people, the agent/memory design for developers, and the Docker
wizard for self-hosters. Keep the cost and privacy caveats in the posts, not just the README.
Check each community's current rules before posting.

## Show HN

**Title:** Show HN: A Discord bot that grows its own personality (Rust)

**Link:** https://github.com/krxi/discord-bot

**First comment (~150 words):**

I made this because I wanted a bot that felt less like a vending machine and more like a
roommate in a group chat. It does not just wait for a slash command: it decides whether to
join a conversation, remembers people, sleeps at night, and eventually picks its own name.

The interesting part for me is the split between talking and learning. The chat path uses a
personality prompt. Background agents do plain analysis: a diarist records people and events,
a coach updates temperament, and a critic leaves corrections. Future replies read those notes.
This changes the context, not the model weights.

It is Rust with serenity and an embedded redb database. Memory is not automatically deleted;
older records are summarized and the raw overflow is archived. Docker and release binaries
have a setup wizard. You bring a model API key. Memory is local, but conversation context
goes to that provider. I'd like feedback on the memory design and conversational pacing.

## r/rust

**Title:** A Rust Discord bot with background agents, redb memory, and a personality that changes

I built a conversational Discord bot with serenity 0.12, tokio, reqwest/rustls, and redb.
Instead of putting all behavior in one static prompt, the talking path reads notes produced
by background agents: group profiling, a conversation diary, temperament updates, and
self-criticism. It advances through growth stages and chooses a name when established.

Some implementation choices that might interest Rust folks:

- Shared chat state uses a `std::sync::Mutex`; the project invariant is no lock held across
  `.await`. Handlers snapshot data, await the model, then update state.
- Analysis and personality-aware generation are separate paths.
- redb stores text values under path-like keys. Retrieval has a character budget; when a
  record is summarized, raw overflow is appended to readable archive files.
- An RAII guard limits each channel to one in-flight reply and clears it on exit or panic.
- Replies use a bounded line protocol: text, silence, or emoji reactions, with streamed output.
- CI checks fmt, clippy, tests, and a Docker build. Tagged releases target Linux x86_64/arm64,
  macOS Apple Silicon, Windows x86_64, and multi-architecture Docker.

This is prompt-and-memory evolution, not fine-tuning. It uses a remote model by default.
I'd welcome criticism of the persistence boundaries and the concurrency design.

Code and beginner instructions: https://github.com/krxi/discord-bot

## r/selfhosted

**Title:** A self-hosted Discord “roommate”: Docker, your own model key, and local memory

I wanted a bot that hangs around a friend group's server instead of only answering commands.
This one remembers people, forms opinions, sleeps on a configurable local schedule, and
later chooses its own name.

The Docker setup is four commands plus a wizard. Think of it as a **60-second launch path
once Docker, a Discord token, and a model key are ready** — not a measured promise about
first-time setup or downloads:

```sh
mkdir mybot && cd mybot
curl -O https://raw.githubusercontent.com/krxi/discord-bot/main/docker-compose.yml
docker run --rm -it --user "$(id -u):$(id -g)" -v "$PWD:/data" ghcr.io/krxi/discord-bot:latest init
docker compose up -d
```

The image supports amd64 and arm64. One Compose service, an embedded redb database, and a
persistent memory volume; no separate database server. BYOK here means **bring your own
OpenRouter or Mistral API key**. No GPU is needed for that setup.

Privacy caveat: saved memory is local, but messages and relevant history/memory go to your
chosen model provider. Background agents also make model calls, so costs and free-model
limits matter. Tell server members what it reads. Back up the volume; do not use `down -v`
unless you want to erase it.

The README includes a click-by-click guide with Windows commands:
https://github.com/krxi/discord-bot

## r/discordapp

**Title:** I made a Discord bot that acts more like a roommate than a command machine

Most bots feel like vending machines: type a command, receive an answer. I wanted something
that could hang out with a group, learn the inside jokes, and decide when to join in.

So I made a bot that remembers people, changes its tone as it gets to know the server,
sleeps at night, and eventually chooses its own name. Sometimes it replies; sometimes it
just leaves an emoji reaction. It can also read the news and comment on images.

It is still a bot, not a person. You run it yourself and give it an AI service key. It supports
English and Turkish, and there is a beginner guide that explains the Discord setup and
Docker commands. Members should know that readable messages can be sent to the AI provider;
it is not a private offline chatbot.

I'd be interested in what would make a server roommate fun without making it intrusive.
Start in a small test channel rather than giving it your whole server on day one.

https://github.com/krxi/discord-bot

## X thread — five posts

**1/5**
Most Discord bots are vending machines: command in, answer out. I made one that's more like
a roommate. It remembers people, sleeps at night, and joins in when it has something to say.
https://github.com/krxi/discord-bot

**2/5**
Its personality changes through memory, not model training. Background agents keep a diary,
update its temperament, and leave notes about its mistakes. The talking path reads those
notes. Eventually, it picks its own name.

**3/5**
Rust + serenity + redb. Saved memory stays on your machine. Older notes are summarized, with
raw overflow archived rather than automatically deleted. A small index and bounded retrieval
feed each reply.

**4/5**
Docker on amd64/arm64, or release binaries. `discord-bot init` asks for settings and prints an
invite link. Bring your own OpenRouter/Mistral key: messages go to that provider, and
background work counts toward usage too.

**5/5**
English and Turkish. Click-by-click beginner guide. MIT licensed. Want to help? Starter tasks:
custom emoji reactions, English travel dates, English sleep keywords, obsession-topic hook.
Try a small channel first; feedback welcome.
https://github.com/krxi/discord-bot

## Manual launch checklist

- [ ] Record the demo GIF with `vhs demo/demo.tape` using a real model key, inspect the output
  for secrets and private text, then add it under the README title in **both** languages.
  The tape exists; a recorded GIF is not included yet. A terminal chat recording demonstrates
  model replies, not Discord willingness/cooldown behavior.
- [ ] Capture a real Discord exchange and a later remembered callback or self-chosen name,
  with members' consent. Label the model used and redact identifying information. Do not
  present a scripted terminal exchange as evidence of autonomous Discord behavior.
- [ ] Upload a social preview image (**Settings → Social preview**). Use a real, redacted
  example or a clear architecture illustration, not invented usage or performance numbers.
- [ ] Confirm the tagged release artifacts and GHCR image are publicly downloadable before
  sharing install commands with new users.
- [ ] Check current subreddit rules and Show HN requirements; adapt each draft to the community.
- [ ] Post Tue–Thu, **14:00–16:00 UTC**. This is a scheduling suggestion, not a proven reach guarantee.
- [ ] Answer comments in the first **2 hours**; explain costs, provider processing, and limits plainly.
- [ ] Pin a **good first issue** list. Create actionable issues for custom emoji reactions,
  an English travel calendar, English sleep weekday/mood keywords, and the obsession keyword hook.
- [ ] Keep English and Turkish READMEs in sync after adding visuals or changing setup instructions.
- [ ] Save real user stories, installation fixes, and personality-change examples. If you
  later write a follow-up, bring new evidence instead of reposting the same promotion.

## Claims deliberately left out

No RAM benchmark, fixed monthly price, “zero telemetry,” credit-card requirement claim,
star-count comparison, or specific free-model recommendation is included. Those need current
measurements or direct evidence. The “roommate” metaphor describes behavior, not consciousness,
perfect recall, privacy from model providers, or a guaranteed level of activity.
