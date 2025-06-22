#!/bin/bash
fbi -a -T 3 -d /dev/fb0 --noverbose /srv/pi-frame/cat-3.png
sleep 1
echo > /sys/class/vtconsole/vtcon1/bind
