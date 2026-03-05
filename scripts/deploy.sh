#!/usr/bin/env bash
set -euo pipefail

PI=root@10.13.1.51
PI_DIR=/root/voice-assistant
IMAGE=ghcr.io/bryanboettcher/wyoming-rust:armv6

echo "==> Building ARMv6 Docker image..."
docker buildx build --platform linux/arm/v7 \
  --build-arg RUST_TARGET=arm-unknown-linux-gnueabihf \
  --build-arg USE_ARMV6_TOOLCHAIN=1 \
  --build-arg LINKER=armv6-rpi-linux-gnueabihf-gcc \
  --build-arg STRIP_CMD=armv6-rpi-linux-gnueabihf-strip \
  --build-arg RUSTFLAGS_EXTRA="-C target-cpu=arm1176jzf-s" \
  --build-arg CROSS_PKG_CONFIG_LIBDIR=/opt/x-tools/armv6-rpi-linux-gnueabihf/armv6-rpi-linux-gnueabihf/sysroot/usr/lib/pkgconfig \
  -t "$IMAGE" --push .

echo "==> Syncing deploy files to Pi..."
scp deploy/satellite.toml "$PI:$PI_DIR/satellite.toml"

echo "==> Pulling and restarting on Pi..."
ssh "$PI" "cd $PI_DIR && docker compose pull satellite && docker compose up -d satellite"

echo "==> Startup logs:"
ssh "$PI" "sleep 3 && docker logs wyoming-satellite --tail 15"
