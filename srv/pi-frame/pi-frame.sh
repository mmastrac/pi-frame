#!/bin/bash
# Handle SIGTERM gracefully by blanking the framebuffer
trap 'gzip -d < /srv/pi-frame/cat.png.bin.gz > /dev/fb0; exit 0' SIGTERM

gzip -d < /srv/pi-frame/cat.png.bin.gz > /dev/fb0
/srv/pi-frame/pi-frame /srv/pi-frame/config.toml
