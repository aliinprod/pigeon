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

## Roadmap
See [ROADMAP.md](ROADMAP.md). Daily progress: [DEVLOG.md](DEVLOG.md).
