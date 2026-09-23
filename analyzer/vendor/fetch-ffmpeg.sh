#!/bin/sh
# Fetch the pinned static ffmpeg/ffprobe into this directory.
#
# Pinned rather than apt-installed: the decoder version is part of the
# determinism guarantee (distro ffmpeg on 22.04 is 4.4.2 and differs elsewhere),
# and it keeps a root-privilege install out of the setup path.
set -eu
cd "$(dirname "$0")"
TAR=ffmpeg-release-amd64-static.tar.xz
curl -sSLO "https://johnvansickle.com/ffmpeg/releases/$TAR"
echo "7fa72b652e19bf84c9461e332ea1cdf3  $TAR" | md5sum -c -
tar xJf "$TAR" --strip-components=1 \
    ffmpeg-7.0.2-amd64-static/ffmpeg ffmpeg-7.0.2-amd64-static/ffprobe
rm "$TAR"
sha256sum -c <<SUMS
e7e7fb30477f717e6f55f9180a70386c62677ef8a4d4d1a5d948f4098aa3eb99  ffmpeg
4f231a1960d83e403d08f7971e271707bec278a9ae18e21b8b5b03186668450d  ffprobe
SUMS
