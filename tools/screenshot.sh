#!/bin/bash
echo "Taking screenshot..."
ffmpeg -loglevel error -f fbdev -framerate 1 -i /dev/fb0 -frames:v 1 -y target/screenshot.png
