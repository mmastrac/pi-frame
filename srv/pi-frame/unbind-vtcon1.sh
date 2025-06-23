#!/bin/bash
gzip -d < /srv/pi-frame/cat-3.png.bin.gz > /dev/fb0
sleep 1
echo > /sys/class/vtconsole/vtcon1/bind
