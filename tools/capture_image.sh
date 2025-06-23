#!/bin/bash
set -euo pipefail

IMAGE="$1"

if [ -z "$IMAGE" ]; then
    echo "Usage: $0 <image>"
    exit 1
fi

OUTNAME=$(basename "$IMAGE")

# Scale the image up by 3x and add rounded corners
# First get the dimensions of the scaled image
SCALED_DIMS=$(convert "$IMAGE" -scale 300% -format "%wx%h" info:)
WIDTH=$(echo $SCALED_DIMS | cut -d'x' -f1)
HEIGHT=$(echo $SCALED_DIMS | cut -d'x' -f2)

ROUNDED_RADIUS=15

# Create a black image with a transparent rounded rectangle cut out @ 50% opacity
convert -size ${WIDTH}x${HEIGHT} xc:black \
  \( -size ${WIDTH}x${HEIGHT} xc:none -fill rgba\(0,0,0,0.5\) \
  -draw "roundrectangle 0,0,${WIDTH},${HEIGHT},${ROUNDED_RADIUS},${ROUNDED_RADIUS}" \) \
  -compose CopyOpacity -composite /tmp/black_mask.png

# Now draw a fully opaque rounded rectangle on the mask, but 10px smaller
SMALLER=10
SMALLER_WIDTH=$((WIDTH-SMALLER))
SMALLER_HEIGHT=$((HEIGHT-SMALLER))
convert /tmp/black_mask.png \
  \( -size ${SMALLER_WIDTH}x${SMALLER_HEIGHT} xc:none -fill black \
    -draw "roundrectangle ${SMALLER},${SMALLER},${SMALLER_WIDTH},${SMALLER_HEIGHT},${ROUNDED_RADIUS},${ROUNDED_RADIUS}" \) \
  -compose SrcOver -composite /tmp/black_mask.png

# Blur the alpha channel of the mask
convert /tmp/black_mask.png -blur 0x10 /tmp/black_mask.png

cp /tmp/black_mask.png target/black_mask.png

# Scale the original image
convert "$IMAGE" -scale 300% /tmp/scaled_image.png

# Composite the black mask (with transparent rounded rectangle) over the scaled image
convert /tmp/scaled_image.png /tmp/black_mask.png -compose CopyOpacity -composite /tmp/"$OUTNAME"

# Replace transparent pixels with black
convert /tmp/"$OUTNAME" -background black -alpha remove -alpha off /tmp/"$OUTNAME"

cp /tmp/"$OUTNAME" target/intermediate.png

sudo fbi -T 3 -d /dev/fb0 --noverbose /tmp/"$OUTNAME"
sudo dd if=/dev/fb0 of=- > /tmp/"$OUTNAME".bin
gzip -9f /tmp/"$OUTNAME".bin
mv /tmp/"$OUTNAME".bin.gz srv/pi-frame/
rm /tmp/"$OUTNAME" /tmp/black_mask.png /tmp/scaled_image.png

echo "Image captured and saved to srv/pi-frame/$OUTNAME.bin.gz"
