# Invisible desktop of the measurement bench (spec 08 §3.2, D1): sway
# without a screen, plus what Cincel, Zed and Electron editors need to run.
# The base image is pinned by digest; packages come from the Ubuntu archive
# (apt checks their signatures and hashes). Nothing here has credentials and
# the container runs with --network none during the measurements.
FROM ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      sway xwayland grim wf-recorder wtype xkb-data \
      mesa-vulkan-drivers libvulkan1 libgl1-mesa-dri libegl1 libgles2 libgbm1 \
      fonts-dejavu-core fontconfig ca-certificates jq git \
      libxkbcommon0 libxkbcommon-x11-0 libxcb1 libx11-xcb1 libasound2t64 \
      libwayland-client0 libwayland-cursor0 libwayland-egl1 \
      libgtk-3-0t64 libnss3 libnspr4 libatk-bridge2.0-0t64 libcups2t64 \
      libxss1 libsecret-1-0 libxkbfile1 libxcomposite1 libxdamage1 libxrandr2 \
      libxshmfence1 libdrm2 libudev1 libatspi2.0-0t64 \
 && rm -rf /var/lib/apt/lists/*

# Everything else (the bench binary, the scripts, the apps and the corpus) is
# mounted read-only by run.sh; the user is the caller's (--user).
CMD ["/bench/tools/entry.sh"]
