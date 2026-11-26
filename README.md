# Dapan 🎀

A cute Vietnamese Discord bot written in Rust.

- 🎶 **Music**: plays YouTube links, playlists or searches, and Spotify tracks, albums and playlists (it finds each Spotify song on YouTube).
- 🗣️ **Text to speech**: reads Vietnamese in voice chat with Microsoft's *HoaiMy* neural voice, pitched up to sound cuter. Music volume drops automatically while it talks.
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

## Commands

| Command | What it does |
|---|---|
| `/play <link or song name>` | Play from YouTube or Spotify (`/p`) |
| `/skip` `/stop` `/pause` `/resume` | Control playback |
| `/queue` `/nowplaying` | Show the queue or the current song |
| `/volume <0-200>` | Set the volume |
| `/join` `/leave` | Join or leave voice (the bot also leaves after 5 minutes idle) |
| `/say <text>` | Speak the text in voice chat |
| `/ttschannel true/false` | Read every message in this channel aloud (while you're in voice) |
| `/weather [city]` | Show the weather (no city = your home city) |
| `/remind <when> <what>` | Set a reminder: `10m`, `2h`, `1h30m`, `14:30`, `mai 8:00`, `25/12 09:00` |
| `/reminders` `/remind-delete <id>` | List or delete your reminders |
| `/calendar` | Show today's and tomorrow's schedule |
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

Code layout: `src/music` (playback and Spotify/YouTube link handling), `src/tts` (voice), `src/weather`, `src/calendar` (iCal + reminders), and `src/scheduler.rs` (background jobs).

### Notes

- **Spotify** doesn't let bots stream its audio, so Dapan reads each song's name from Spotify and plays it from YouTube. Spotify-made playlists (such as *Today's Top Hits*) can't be read with an app key.
- **TTS**: Microsoft's service drops the connection when pitch is set in the request, so Dapan requests the normal voice and raises the pitch with FFmpeg instead (`TTS_PITCH`).
- **`vendor/msedge-tts`** is a patched copy of the `msedge-tts` crate. It sends timestamps and cookies the same way the reference *edge-tts* client does. Without that, Microsoft's service randomly refuses about half of all requests. Search for "Patched for dapan" to see the changes.
- **iCal recurring events**: DAILY, WEEKLY (with BYDAY), MONTHLY and YEARLY are supported, along with INTERVAL, COUNT, UNTIL, EXDATE and edited single occurrences. More complex rules (for example "second Tuesday of the month") only notify for the first occurrence.
