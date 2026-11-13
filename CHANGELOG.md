# Changelog

All notable changes to Citylive 2. The release workflow publishes the section of the tagged version as the
GitHub release notes, so each release needs a `## [x.y.z] - date` heading here before it is tagged.

## [0.2.0] - 2026-11-13

First public release.

- A living isometric pixel-art city on your desktop: it founds a settlement, grows it through the ages, and has
  seasons, weather, day and night, the real sun, moon and stars, holidays, wildlife, traffic and trains.
- Windows 11: a native wallpaper that sits behind the icons on every monitor, with a tray icon, pause while a
  full-screen app is in front, and a per-user installer (Start menu, optional start with Windows, uninstaller).
- Linux: KDE Plasma (through a native wallpaper module), GNOME, X11 and wlroots compositors, installed per user
  with a systemd user service by `install.sh`, or system-wide from the `.deb`.
- Location: the operating system's location service (Windows, GeoClue), then a free IP lookup, then the place you
  type in the settings window. Weather comes from Open-Meteo.
- Settings window with a live preview, a place search, and personal dates.
- Automatic updates: the program looks at the GitHub releases at start and every six hours, downloads a newer
  version in the background, checks it against the release's `SHA256SUMS`, and installs it the next time it starts.
  It can be turned off in the settings.
