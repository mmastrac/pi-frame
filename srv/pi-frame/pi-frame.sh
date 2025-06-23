#!/bin/bash
gzip -d < /srv/pi-frame/cat.png.bin.gz > /dev/fb0
/srv/pi-frame/pi-frame /srv/pi-frame/config.toml

