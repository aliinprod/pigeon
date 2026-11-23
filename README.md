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
   Keep it running. The first start creates the Mac's device key in your login Keychain (macOS may ask you to allow access; choose "Always Allow").
3. **Pair the home PC (once).** In a second Mac terminal:
   ```sh
   ./pigeon-agent pair --name "Home PC"
   ```
   It prints an **invite link** (`pigeon-invite-...`, not secret: you may email it to yourself) and a **4-word code** like `otter-plaza-crumb-tulip`. The code is the secret: write it down or take a photo of the screen, never send it. The invite works once, for 12 hours, and dies after 3 wrong codes.
4. **On Windows**, pair with the link and type the code when asked:
   ```powershell
   .\pigeon.exe pair pigeon-invite-... --name "Office Mac"
   ```
   Both sides show each other's fingerprint. From now on no ticket or code is needed: the Mac recognises this PC by its key (kept in Windows Credential Manager).
5. **List and fetch:**
   ```powershell
   .\pigeon.exe ls
   .\pigeon.exe get report.pdf
   ```
   `get` resumes an interrupted download, checks the SHA-256, and never overwrites: if `report.pdf` exists it saves `report (1).pdf` and prints the final path.
   Add `--relay-only` to `ls` or `get` to force the encrypted relay path (slower; for testing). Add `-v` to see the full error details if something fails.

**Lost the PC, or done with it?** On the Mac: `./pigeon-agent devices` lists paired devices and `./pigeon-agent revoke "Home PC"` cuts one off at once (a running agent drops its connections within a second). `./pigeon-agent reset --yes` makes a new Mac key and forgets every pairing (restart a running `share` afterwards, then pair again).

The Day-1 "ticket" is gone: anyone who ever saw one can no longer use it.

## Roadmap
See [ROADMAP.md](ROADMAP.md). Daily progress: [DEVLOG.md](DEVLOG.md).
