# Security policy

faro keeps your Signal account keys, your message history and your attachments on your computer. If you find a way to get at them, or anything else that puts faro's users at risk, please report it privately so it can be fixed before it's public.

## Reporting a vulnerability

**[Report it privately on GitHub](https://github.com/harababurel/faro/security/advisories/new)** (the repository's Security tab › *Report a vulnerability*). Only the maintainer sees the report.

Please don't open a public issue or pull request for a security problem, or post it elsewhere first.

A useful report has:

- the faro version (`faro --version`) and your OS and terminal
- what an attacker can do, and what they need to start with (send you a message, a file, a link…)
- steps to reproduce, ideally in `faro demo` if the problem doesn't need a real account

## What to expect

faro is maintained by one person in their spare time, and there's no bug bounty. You can expect:

- an acknowledgement within a week
- an honest assessment, and a fix in a new release as soon as is reasonable for its severity
- credit in the advisory and the release notes, unless you'd rather not be named

Please give a fix a reasonable time (up to 90 days) before disclosing it publicly. We can agree on a date together.

## Supported versions

Only the latest release gets security fixes. Packages (Homebrew, the AUR, the `.deb`) follow the releases.

| Version | Supported |
|---|---|
| latest 0.1.x | ✅ |
| older | ❌ |

## Scope

**In scope:** faro's own code and the website.

- Local data: account keys, history, attachments or logs readable by other users, or message text or phone numbers ending up in the logs
- Terminal output: messages, names or file names that can inject escape sequences into your terminal, change its title or clipboard, or forge what faro shows
- Files and programs: saving or opening attachments outside where they should go, or running external programs (players, openers, transcription) unsafely
- Data leaving your machine other than to Signal's servers, such as remote transcription without your consent
- Sending messages, read receipts or typing indicators you didn't ask for
- getfaro.dev and its in-browser demo (for example, cross-site scripting, or anything typed into the demo leaving the browser)

**Report upstream instead:**

- the Signal protocol, libsignal and Signal's servers: Signal, at [security@signal.org](mailto:security@signal.org)
- [presage](https://github.com/whisperfish/presage), which faro uses to talk to Signal: the presage project

faro implements no cryptography of its own. If you're not sure where a problem belongs, report it here and it will be passed on.

**Not security issues:** faro stopping working after a change on Signal's side, problems that need an attacker who already controls your user account or terminal, and the documented limits (for example, the data [isn't encrypted at rest yet](README.md#privacy-and-limits)).
