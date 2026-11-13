Citylive 2 - Linux (x86_64)

Install (per user, no root):   sh install.sh
Remove:                        sh ~/.local/share/citylive-2/uninstall.sh

What you get: ~/.local/share/citylive-2 (the programs), a systemd user service
"citylive-2.service" that starts the wallpaper with your graphical session, and a
"Citylive 2 Settings" entry in the application menu. Your settings and saved
worlds live in ~/.config/citylive-iso and ~/.local/share/citylive-iso.

KDE Plasma: the installer offers to build the Plasma wallpaper module (needs cmake,
g++ and the Qt 6 development packages) and to make Citylive 2 your wallpaper.
GNOME and others: the presenter for your session is chosen automatically.

The program checks GitHub for a new release at start and every six hours and installs
it the next time it starts (turn it off in the settings window).

https://github.com/Deluxescout1/citylive-2
