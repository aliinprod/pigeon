<p align="center"><img src="brand/logo.webp" width="140" alt="Pigeon logo"></p>

<h1 align="center">Pigeon</h1>
<p align="center"><b>Your files know the way home.</b></p>

Fetch files from your work computer while you're at home. No VPN, no open ports, no cloud copy of your files.

> Status: early, built in public. Expect rough edges.

## How it works
1. **Agent** runs on your work Mac and shares only the folders you choose.
2. **App** on your home PC asks for a file.
3. Both connect *outward* to a relay that introduces them. They talk directly when they can; when a firewall blocks that, the relay forwards encrypted bytes it can't read.

Files stay on the Mac. Only the copy you fetch lands at home.

## Try it
Early builds are unsigned and come from GitHub Actions (repo access needed).

1. **Download** the latest run of the [`binaries` workflow](../../actions/workflows/binaries.yml):
   `pigeon-macos-universal` for the Mac (Apple Silicon + Intel) and `pigeon-windows-x64` for the PC.
2. **On the Mac**, unpack and share a folder:
   ```sh
   tar xzf pigeon-macos-universal.tar.gz && cd pigeon-macos-universal
   # Unsigned binaries: clear the quarantine flag once (or right-click -> Open in Finder).
   xattr -d com.apple.quarantine pigeon-agent pigeon
   ./pigeon-agent share ~/Documents/ForHome
   ```
   It prints a `ticket: pigeon...` line. Keep the agent running, and treat the ticket like a password: anyone with it can read that folder.
3. **On Windows**, list and fetch:
   ```powershell
   .\pigeon.exe ls <TICKET>
   .\pigeon.exe get <TICKET> report.pdf
   ```
   `get` resumes an interrupted download, checks the SHA-256, and never overwrites: if `report.pdf` exists it saves `report (1).pdf` and prints the final path.
   Add `--relay-only` to force the encrypted relay path (slower; for testing).

## Roadmap
See [ROADMAP.md](ROADMAP.md). Daily progress: [DEVLOG.md](DEVLOG.md).
