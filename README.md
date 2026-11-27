# Dapan 🎀

A cute Vietnamese Discord bot written in Rust.

- 🎶 **Music**: plays YouTube links, playlists or searches, and Spotify tracks, albums and playlists (it finds each Spotify song on YouTube).
- 🗣️ **Text to speech**: reads Vietnamese in voice chat with a local AI voice on your GPU ([VieNeu-TTS](https://github.com/pnnbao97/VieNeu-TTS), free), or Microsoft's *HoaiMy* voice as a fallback. Long text is read sentence by sentence, and music volume drops while it talks.
- 📊 **Task tracking**: `/tasks` shows what Dapan is doing and how long each step took.
- ☀️ **Weather**: `/weather`, a morning forecast, and alerts for heavy rain, storms or heat (Open-Meteo, no key needed).
- 📅 **Calendar**: reminds you before events in Google Calendar, plus your own `/remind` reminders.

## 1. Install the tools (Windows)

```powershell
winget install Rustlang.Rustup Kitware.CMake yt-dlp.yt-dlp Gyan.FFmpeg
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
# Recommended: yt-dlp uses Deno to read YouTube reliably
winget install DenoLand.Deno --source winget
```

Open a **new** terminal after installing so the PATH updates. Keep yt-dlp up to date, because YouTube changes often: `yt-dlp -U` (or `winget upgrade yt-dlp.yt-dlp`).

## 2. Create the Discord bot

1. Go to https://discord.com/developers/applications, click **New Application**, open **Bot**, and click **Reset Token**. Copy the token.
2. On the same page, turn on **Message Content Intent** (needed to read the TTS channel aloud).
3. Under **OAuth2 → URL Generator**, select the scopes `bot` and `applications.commands`. Then select the permissions *View Channels, Send Messages, Embed Links, Add Reactions, Connect, Speak*. Open the generated link and invite the bot to your server.
4. To get IDs: in Discord, go to Settings → Advanced and turn on Developer Mode. Then right-click a server, channel or user and choose **Copy ID**.

## 3. Configure

```powershell
copy .env.example .env
notepad .env
```

Only `DISCORD_TOKEN` is required. Set `GUILD_ID` so slash commands appear immediately.

- **Google Calendar**: open Google Calendar settings, choose your calendar, and copy **Secret address in iCal format** into `ICAL_URL`.
- **Spotify playlists**: create an app at https://developer.spotify.com/dashboard and copy its Client ID and Secret.

## 4. Run

```powershell
cargo run --release
```

## 5. Keep it running (starts with Windows, in the background)

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install-autostart.ps1
```

Windows asks for administrator permission (UAC); click **Yes**. The script creates two scheduled tasks, **Dapan Discord Bot** and **Dapan TTS Server** (the second only if the local voice is installed). They:
- **start when Windows boots**, even before you log in, and keep running after you log out.
- **run in the background** in a separate Windows session, so no console window ever appears. Windows runs them as your user without storing your password.

Each task runs a script (`scripts\run-dapan.ps1` or `scripts\run-tts-server.ps1`) that:
- restarts the program whenever it stops. After a quick crash it waits 10 s, then 20 s, and so on, up to 5 minutes; after a normal run it restarts in 5 s.
- writes everything to `logs\dapan-YYYY-MM-DD.log` and `logs\tts-YYYY-MM-DD.log`, and deletes logs older than 14 days.
- builds Dapan if the binary is missing, and waits for `.env` if it hasn't been created yet.
- runs only one copy at a time.

Day-to-day commands (each asks for administrator permission):

```powershell
Get-Content logs\dapan-$(Get-Date -f yyyy-MM-dd).log -Tail 30 -Wait          # watch the log (no admin needed)
powershell -ExecutionPolicy Bypass -File scripts\restart-dapan.ps1           # restart, e.g. after editing .env
powershell -ExecutionPolicy Bypass -File scripts\restart-dapan.ps1 -Build    # rebuild + restart after changing code
powershell -ExecutionPolicy Bypass -File scripts\stop-dapan.ps1              # stop until the next boot
powershell -ExecutionPolicy Bypass -File scripts\uninstall-autostart.ps1     # remove the tasks
```

Set Windows Sleep to **Never** (Settings → System → Power) so the bot stays online.

**`.env` format:** if a value contains spaces, put it in double quotes, for example `HOME_CITY="Hà Nội"`. Dapan refuses to start with a clear error message if a line can't be read. Otherwise every setting after a bad line would be silently ignored.

## 6. Local voice (VieNeu-TTS, optional but recommended)

Microsoft's free voice is slow for long text. It generates audio at about playback speed, and it sometimes throttles. VieNeu-TTS v3 Turbo is a free (Apache 2.0) Vietnamese model that runs on your NVIDIA GPU, so speech starts almost instantly. It needs an NVIDIA driver 570 or newer, and about 6 GB of disk space.

```powershell
winget install Python.Python.3.11
cd tts-server
py -3.11 -m venv .venv
.venv\Scripts\python -m pip install torch==2.8.0 torchaudio==2.8.0 --index-url https://download.pytorch.org/whl/cu128
.venv\Scripts\python -m pip install "transformers==4.57.6" vieneu
.venv\Scripts\python samples.py      # optional: a sample of every voice in tts-server\samples
```

Then add these to `.env` and run `scripts\install-autostart.ps1` again, which also adds the **Dapan TTS Server** background task:

```
TTS_LOCAL_URL=http://127.0.0.1:5005
TTS_LOCAL_VOICE="Trúc Ly"
TTS_PITCH=0
```

- **Voices:** VieNeu has 25 preset voices (North, Central and South), for example *Trúc Ly* (default), *Mai Anh*, *Ngọc Huyền* and *Thùy Dung*. Listen to the files in `tts-server\samples` and put your favorite in `TTS_LOCAL_VOICE`. `TTS_PITCH` and `TTS_SPEED` still work on top if you want it cuter.
- **Your own voice:** put a clean 3–8 second clip at `tts-server\voices\<name>.wav`, restart the TTS server, and set `TTS_LOCAL_VOICE=<name>`. Only clone voices you have permission to use.
- **Fallback:** if the server is down or fails, Dapan uses Microsoft's voice automatically. `/tasks` shows which engine spoke ("local" or "Edge").
- **Logs:** the TTS server logs to `logs\tts-YYYY-MM-DD.log`.

## 7. Invite Dapan to other servers (friends)

1. In `.env`, leave `GUILD_ID=` empty. Dapan then registers its commands for every server and removes the old copies from your server, so nothing shows twice. Set `OWNER_USER_ID` to your own user ID. Restart Dapan.
2. In the Developer Portal, open your app, go to **Bot**, and turn on **Public Bot**.
3. Share this link. Replace `<CLIENT_ID>` with your Application ID from the Developer Portal's **General Information** page:
   `https://discord.com/oauth2/authorize?client_id=<CLIENT_ID>&scope=bot+applications.commands&permissions=3230784`

The person adding it needs **Manage Server** permission. New commands can take a few minutes to appear; Ctrl+R in Discord reloads them.

What's shared and what's private:
- Music, TTS, `/weather` and `/remind` work for everyone.
- `/calendar` shows your Google Calendar only to you (`OWNER_USER_ID`). Everyone else sees only their own reminders, and the reply is private.
- The morning brief and weather/calendar alerts go only to you.

## 8. Deploy to a VPS or AWS (instead of your PC)

| Option | Cost | Notes |
|---|---|---|
| Hetzner / Vultr / DigitalOcean (Ubuntu, 2 GB RAM) | ~$4–6/month | Simplest, best value |
| AWS Lightsail | $5/month (2 GB) | AWS's simple option |
| AWS EC2 t3.micro / t4g.small | Free tier 12 months, then ~$8–12 | The free tier has 1 GB RAM: add swap space before building Rust |

Steps on Ubuntu:

```bash
sudo apt update && sudo apt install -y build-essential cmake pkg-config ffmpeg git curl
curl https://sh.rustup.rs -sSf | sh -s -- -y && source ~/.cargo/env
sudo curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp && sudo chmod +x /usr/local/bin/yt-dlp
curl -fsSL https://deno.land/install.sh | sh   # yt-dlp uses Deno for YouTube
git clone https://github.com/<you>/dapan.git && cd dapan
cargo build --release
```

Then:
1. Copy `.env` (and `data/dapan.db` to keep reminders) from your PC with `scp`, not through git.
2. Create `/etc/systemd/system/dapan.service`:

```ini
[Unit]
Description=Dapan Discord bot
After=network-online.target

[Service]
WorkingDirectory=/home/ubuntu/dapan
ExecStart=/home/ubuntu/dapan/target/release/dapan
Restart=always
RestartSec=5
Environment=PATH=/usr/local/bin:/usr/bin:/bin:/home/ubuntu/.deno/bin

[Install]
WantedBy=multi-user.target
```

3. Start it: `sudo systemctl enable --now dapan`. Watch the log with `journalctl -u dapan -f`.

Watch out for:
- **Run only one copy per token.** Stop the PC version first (`scripts\uninstall-autostart.ps1`); two copies would answer every command twice.
- **YouTube often blocks server IPs** ("Sign in to confirm you're not a bot"). Home internet avoids this. On a server, yt-dlp needs your YouTube cookies.
- **Local voice on a server** needs a GPU instance, which costs much more. Leave `TTS_LOCAL_URL` empty there and Dapan uses Microsoft's voice.
- **Firewall:** voice only uses outgoing connections, so only SSH needs to be open.

## Commands

| Command | What it does |
|---|---|
| `/play <link or song name>` | Play from YouTube or Spotify (`/p`) |
| `/skip` `/stop` `/pause` `/resume` | Control playback |
| `/queue [page]` `/nowplaying` | Show the queue (10 songs per page, with loop/autoplay status) or the current song |
| `/remove <n>` `/move <from> <to>` `/skipto <n>` | Remove, reorder, or jump to song *n* (numbers from `/queue`) |
| `/shuffle` `/clear` | Shuffle the songs waiting, or remove them (the current song keeps playing) |
| `/loop off/track/queue` | Repeat nothing, the current song (until `/skip`), or the whole queue |
| `/autoplay [on/off]` | When one song is left, Dapan queues a related one from YouTube's Mix (no repeats of recent songs, nothing over 15 min) and posts it here (`/ap`) |
| `/volume <0-200>` | Set the volume |
| `/join` `/leave` | Join or leave voice (the bot also leaves after 5 minutes idle) |
| `/say <text>` | Speak the text in voice chat |
| `/ttschannel true/false` | Read every message in this channel aloud (while you're in voice) |
| `/weather [city]` | Show the weather (no city = your home city) |
| `/remind <when> <what>` | Set a reminder: `10m`, `2h`, `1h30m`, `14:30`, `mai 8:00`, `25/12 09:00` |
| `/reminders` `/remind-delete <id>` | List or delete your reminders |
| `/calendar` | Show today's and tomorrow's schedule (private) |
| `/tasks` | What Dapan is doing now, and how long recent tasks took, step by step (🐢 = slower than 5 s) |
| `/help` `/ping` | |

## Automatic notifications

These go to `NOTIFY_CHANNEL_ID` and/or a DM to `OWNER_USER_ID`:

- **Every morning** at `MORNING_TIME`: the weather forecast and today's schedule.
- **Every hour**: a check of the next 3 hours for heavy rain (≥10 mm/h), storms or heat (≥37°C). Each kind of alert is sent at most once a day.
- **Calendar**: a reminder `REMIND_BEFORE_MIN` minutes before each event, and another when it starts.

## Development

```powershell
cargo test                       # offline unit tests
cargo test -- --include-ignored  # also tests Edge TTS + Open-Meteo for real (needs internet + ffmpeg)
cargo clippy
```

Code layout: `src/music` (playback and Spotify/YouTube link handling), `src/tts` (voice: `local.rs` VieNeu client, `edge.rs` Microsoft, `split.rs` sentence splitting), `src/weather`, `src/calendar` (iCal + reminders), `src/tracker.rs` (/tasks timings), `src/scheduler.rs` (background jobs), and `tts-server/` (the Python VieNeu server).

### Notes

- **Spotify** doesn't let bots stream its audio, so Dapan reads each song's name from Spotify and plays it from YouTube. Spotify-made playlists (such as *Today's Top Hits*) can't be read with an app key.
- **TTS**: Microsoft's service drops the connection when pitch is set in the request, so Dapan requests the normal voice and raises the pitch with FFmpeg instead (`TTS_PITCH`).
- **Long text**: up to 2000 characters. Text is split into sentences of up to about 200 characters. The next sentence is generated while the current one plays, so there's no long silence and nothing is cut off.
- **`vendor/msedge-tts`** is a patched copy of the `msedge-tts` crate. It sends timestamps and cookies the same way the reference *edge-tts* client does. Without that, Microsoft's service randomly refuses about half of all requests. Search for "Patched for dapan" to see the changes.
- **iCal recurring events**: DAILY, WEEKLY (with BYDAY), MONTHLY and YEARLY are supported, along with INTERVAL, COUNT, UNTIL, EXDATE and edited single occurrences. More complex rules (for example "second Tuesday of the month") only notify for the first occurrence.
