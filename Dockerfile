# Unified multi-architecture Dockerfile for wyoming-satellite
#
# Supports native builds and cross-compilation to multiple ARM targets.
# Uses build ARGs to configure toolchain installation dynamically.
#
# The builder stage always runs on the CI runner's native platform ($BUILDPLATFORM)
# and cross-compiles via Rust's target system. The runtime stage uses $TARGETPLATFORM
# to pull the correct architecture variant of debian:bookworm-slim, ensuring the
# final image has correct platform metadata for Docker manifest matching.
#
# ARMv6 (Pi Zero W) special handling:
# Debian does not publish linux/arm/v6 images, so the runtime uses linux/arm/v7
# (armhf). However, Debian's arm/v7 system libraries (glibc, libasound2, etc.)
# contain ARMv7 instructions that SIGILL on ARMv6 hardware. To fix this, the
# builder stage prepares a /runtime-libs/ directory containing ARMv6-compiled
# shared libraries from the tttapa cross-toolchain sysroot plus our cross-compiled
# alsa-lib. These are overlaid onto the runtime image, replacing the arm/v7 libs
# with ARMv6-safe equivalents. The ld.so.cache is then regenerated to pick up the
# new libraries.
#
# IMPORTANT: All builds should use `docker buildx build --platform <target>` to set
# $BUILDPLATFORM and $TARGETPLATFORM automatically.
#
# Build Examples:
#
# Native x86_64 (default):
#   docker buildx build --platform linux/amd64 \
#     -t wyoming-satellite:x86_64 .
#
# ARMv6 (Raspberry Pi Zero W v1.1):
#   docker buildx build --platform linux/arm/v7 \
#     --build-arg RUST_TARGET=arm-unknown-linux-gnueabihf \
#     --build-arg USE_ARMV6_TOOLCHAIN=1 \
#     --build-arg LINKER=armv6-rpi-linux-gnueabihf-gcc \
#     --build-arg STRIP_CMD=armv6-rpi-linux-gnueabihf-strip \
#     --build-arg RUSTFLAGS_EXTRA="-C target-cpu=arm1176jzf-s" \
#     --build-arg CROSS_PKG_CONFIG_LIBDIR=/opt/x-tools/armv6-rpi-linux-gnueabihf/armv6-rpi-linux-gnueabihf/sysroot/usr/lib/pkgconfig \
#     -t wyoming-satellite:armv6 .
#
# ARMv7 (Raspberry Pi 2/3/4):
#   docker buildx build --platform linux/arm/v7 \
#     --build-arg RUST_TARGET=armv7-unknown-linux-gnueabihf \
#     --build-arg CROSS_PKG="gcc-arm-linux-gnueabihf libc6-dev-armhf-cross libasound2-dev:armhf" \
#     --build-arg LINKER=arm-linux-gnueabihf-gcc \
#     --build-arg STRIP_CMD=arm-linux-gnueabihf-strip \
#     --build-arg CROSS_PKG_CONFIG_LIBDIR=/usr/lib/arm-linux-gnueabihf/pkgconfig \
#     -t wyoming-satellite:armv7 .
#
# ARM64 (Raspberry Pi 3/4/5):
#   docker buildx build --platform linux/arm64 \
#     --build-arg RUST_TARGET=aarch64-unknown-linux-gnu \
#     --build-arg CROSS_PKG="gcc-aarch64-linux-gnu libc6-dev-arm64-cross libasound2-dev:arm64" \
#     --build-arg LINKER=aarch64-linux-gnu-gcc \
#     --build-arg STRIP_CMD=aarch64-linux-gnu-strip \
#     --build-arg CROSS_PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig \
#     -t wyoming-satellite:arm64 .
#
# i686 (32-bit x86):
#   docker buildx build --platform linux/386 \
#     --build-arg RUST_TARGET=i686-unknown-linux-gnu \
#     --build-arg CROSS_PKG="gcc-multilib libc6-dev-i386 libasound2-dev:i386" \
#     --build-arg CROSS_PKG_CONFIG_LIBDIR=/usr/lib/i386-linux-gnu/pkgconfig \
#     -t wyoming-satellite:i686 .

# ---------------------------------------------------------------------------
# Builder Stage
# ---------------------------------------------------------------------------
# Always run the builder on the CI runner's native platform (amd64) since
# we cross-compile via Rust/cargo, not via QEMU emulation.
FROM --platform=$BUILDPLATFORM rust:1.84-bookworm AS builder

ARG RUST_TARGET=x86_64-unknown-linux-gnu
ARG LINKER=""
ARG STRIP_CMD=strip
ARG RUSTFLAGS_EXTRA=""
ARG CROSS_PKG=""
ARG USE_ARMV6_TOOLCHAIN=""
ARG CROSS_PKG_CONFIG_LIBDIR=""

# Install native ALSA dev headers (needed for native builds and as fallback)
RUN apt-get update \
    && apt-get install -y --no-install-recommends libasound2-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Install ARMv6 toolchain (Pi Zero W) if requested
# Debian/Ubuntu's gcc-arm-linux-gnueabihf defaults to ARMv7, which won't
# run on Pi Zero W (ARM1176JZF-S core). We use tttapa's ARMv6 toolchain.
RUN if [ -n "$USE_ARMV6_TOOLCHAIN" ]; then \
        apt-get update \
        && apt-get install -y --no-install-recommends \
            wget \
            xz-utils \
            ca-certificates \
        && rm -rf /var/lib/apt/lists/* \
        && wget -q https://github.com/tttapa/docker-arm-cross-toolchain/releases/download/1.1.0/x-tools-armv6-rpi-linux-gnueabihf-gcc13.tar.xz \
        && echo "93ded492b01386884421fe3f420ccd9af21535ef88f89dfec745782532c40247  x-tools-armv6-rpi-linux-gnueabihf-gcc13.tar.xz" | sha256sum -c - \
        && tar -xJf x-tools-armv6-rpi-linux-gnueabihf-gcc13.tar.xz -C /opt \
        && rm x-tools-armv6-rpi-linux-gnueabihf-gcc13.tar.xz; \
    fi

# Add ARMv6 toolchain to PATH if installed
ENV PATH="${USE_ARMV6_TOOLCHAIN:+/opt/x-tools/armv6-rpi-linux-gnueabihf/bin:}${PATH}"

# Cross-compile alsa-lib from source for ARMv6 toolchain
# This ensures libasound.so is built against the tttapa toolchain's glibc
# instead of Debian's glibc 2.36, which the tttapa sysroot doesn't provide.
RUN if [ -n "$USE_ARMV6_TOOLCHAIN" ]; then \
        apt-get update \
        && apt-get install -y --no-install-recommends \
            make \
            autoconf \
            automake \
            libtool \
        && rm -rf /var/lib/apt/lists/* \
        && cd /tmp \
        && wget -q https://www.alsa-project.org/files/pub/lib/alsa-lib-1.2.12.tar.bz2 \
        && tar -xjf alsa-lib-1.2.12.tar.bz2 \
        && cd alsa-lib-1.2.12 \
        && ./configure \
            --host=armv6-rpi-linux-gnueabihf \
            --prefix=/opt/x-tools/armv6-rpi-linux-gnueabihf/armv6-rpi-linux-gnueabihf/sysroot/usr \
            --disable-python \
        && make -j$(nproc) \
        && make install \
        && cd /tmp \
        && rm -rf alsa-lib-1.2.12 alsa-lib-1.2.12.tar.bz2; \
    fi

# Install standard cross-compiler packages and cross-arch libraries if specified.
# For cross-compilation, CROSS_PKG should include arch-qualified ALSA dev packages
# (e.g., libasound2-dev:armhf for ARM targets).
# NOTE: For ARMv6, we DO NOT install libasound2-dev:armhf because we build it from source.
RUN if [ -n "$CROSS_PKG" ]; then \
        dpkg --add-architecture armhf 2>/dev/null; \
        dpkg --add-architecture arm64 2>/dev/null; \
        dpkg --add-architecture i386 2>/dev/null; \
        apt-get update \
        && apt-get install -y --no-install-recommends $CROSS_PKG \
        && rm -rf /var/lib/apt/lists/*; \
    fi

# Add the Rust target (skip for native x86_64)
RUN if [ "$RUST_TARGET" != "x86_64-unknown-linux-gnu" ]; then \
        rustup target add $RUST_TARGET; \
    fi

WORKDIR /build

# Create .cargo/config.toml with linker and rustflags if needed
RUN mkdir -p .cargo && { \
        echo "[target.$RUST_TARGET]"; \
        if [ -n "$LINKER" ]; then \
            echo "linker = \"$LINKER\""; \
        fi; \
        if [ -n "$RUSTFLAGS_EXTRA" ]; then \
            printf 'rustflags = ['; \
            first=1; \
            for flag in $RUSTFLAGS_EXTRA; do \
                if [ "$first" = 1 ]; then first=0; else printf ', '; fi; \
                printf '"%s"' "$flag"; \
            done; \
            echo ']'; \
        fi; \
    } > .cargo/config.toml

# Enable cross-compilation for pkg-config (ALSA, etc.)
# CROSS_PKG_CONFIG_LIBDIR overrides pkg-config's search path so it finds
# target-arch .pc files instead of the host x86_64 ones. Without this, the
# linker gets -L /usr/lib/x86_64-linux-gnu and fails on cross-arch libc.
#
# IMPORTANT: We must NOT set PKG_CONFIG_LIBDIR to an empty string — that tells
# pkg-config to search NO directories, breaking even native builds where
# libasound2-dev is installed normally. Instead of using ENV (which would bake
# an empty string into the image), each cargo RUN step below conditionally
# exports PKG_CONFIG_LIBDIR only when CROSS_PKG_CONFIG_LIBDIR is non-empty.
ENV PKG_CONFIG_ALLOW_CROSS=1

# -- Dependency caching layer --
COPY Cargo.toml Cargo.lock ./
COPY crates/wyoming/Cargo.toml crates/wyoming/Cargo.toml
COPY crates/satellite/Cargo.toml crates/satellite/Cargo.toml

# Create stub sources to let cargo resolve and build dependencies
RUN if [ -n "$CROSS_PKG_CONFIG_LIBDIR" ]; then export PKG_CONFIG_LIBDIR="$CROSS_PKG_CONFIG_LIBDIR"; fi \
    && mkdir -p crates/wyoming/src crates/satellite/src \
    && echo "pub fn stub() {}" > crates/wyoming/src/lib.rs \
    && echo "fn main() {}" > crates/satellite/src/main.rs \
    && if [ "$RUST_TARGET" = "x86_64-unknown-linux-gnu" ]; then \
        cargo build --release --workspace 2>&1; \
    else \
        cargo build --release --target $RUST_TARGET --workspace 2>&1; \
    fi \
    && rm -rf crates/wyoming/src crates/satellite/src

# -- Real source build --
COPY crates/ crates/

# Remove stale fingerprints and build with real sources
RUN if [ -n "$CROSS_PKG_CONFIG_LIBDIR" ]; then export PKG_CONFIG_LIBDIR="$CROSS_PKG_CONFIG_LIBDIR"; fi \
    && if [ "$RUST_TARGET" = "x86_64-unknown-linux-gnu" ]; then \
        rm -f target/release/wyoming-satellite target/release/libwyoming.rlib \
        && rm -rf target/release/.fingerprint/wyoming-* \
        && cargo test --workspace \
        && cargo build --release --workspace; \
    else \
        rm -f target/$RUST_TARGET/release/wyoming-satellite \
        && rm -rf target/$RUST_TARGET/release/.fingerprint/wyoming-* \
        && cargo build --release --target $RUST_TARGET --workspace; \
    fi

# Strip the binary
RUN if [ "$RUST_TARGET" = "x86_64-unknown-linux-gnu" ]; then \
        $STRIP_CMD target/release/wyoming-satellite; \
    else \
        $STRIP_CMD target/$RUST_TARGET/release/wyoming-satellite; \
    fi

# Consolidate binary to a known location for easy copying
RUN mkdir -p /output && \
    if [ "$RUST_TARGET" = "x86_64-unknown-linux-gnu" ]; then \
        cp target/release/wyoming-satellite /output/wyoming-satellite; \
    else \
        cp target/$RUST_TARGET/release/wyoming-satellite /output/wyoming-satellite; \
    fi

# Prepare ARMv6 runtime libraries overlay
# For ARMv6 builds, copy the tttapa sysroot's shared libraries (glibc, libm,
# libpthread, ld-linux-armhf, nss modules, etc.) plus our cross-compiled
# libasound into Debian's armhf multiarch paths. These will overlay the arm/v7
# libraries in the runtime image, replacing them with ARMv6-safe equivalents.
# Always create /runtime-libs so the COPY in the runtime stage succeeds for all targets.
#
# IMPORTANT: Debian bookworm uses usrmerge — /lib is a symlink to /usr/lib.
# BuildKit COPY cannot write into symlink targets, so all libs must go under
# /usr/lib/arm-linux-gnueabihf/ (the real directory), not /lib/arm-linux-gnueabihf/.
RUN mkdir -p /runtime-libs && \
    if [ -n "$USE_ARMV6_TOOLCHAIN" ]; then \
        SYSROOT=/opt/x-tools/armv6-rpi-linux-gnueabihf/armv6-rpi-linux-gnueabihf/sysroot \
        && mkdir -p /runtime-libs/usr/lib/arm-linux-gnueabihf \
        && cp -a $SYSROOT/lib/*.so* /runtime-libs/usr/lib/arm-linux-gnueabihf/ \
        && cp -a $SYSROOT/usr/lib/libasound.so* /runtime-libs/usr/lib/arm-linux-gnueabihf/ \
        && LIBGCC=$(find /opt/x-tools/armv6-rpi-linux-gnueabihf -name 'libgcc_s.so.1' -print -quit) \
        && if [ -n "$LIBGCC" ]; then \
            echo "Found libgcc_s.so.1 at: $LIBGCC" \
            && cp -aL "$LIBGCC" /runtime-libs/usr/lib/arm-linux-gnueabihf/; \
        else \
            echo "WARNING: libgcc_s.so.1 not found in toolchain!" >&2 && exit 1; \
        fi \
        && mkdir -p /runtime-libs/etc \
        && : > /runtime-libs/etc/ld.so.cache; \
    fi

# ---------------------------------------------------------------------------
# Runtime Stage
# ---------------------------------------------------------------------------
# $TARGETPLATFORM is set by buildx when --platform is passed to the build.
# This ensures the final image carries correct platform metadata so Docker
# can match it on the target hardware.
#
# For ARMv6 builds, the platform is linux/arm/v7 (Debian's armhf). The arm/v7
# system libraries would normally SIGILL on ARMv6 hardware, but the COPY of
# /runtime-libs/ below overlays ARMv6-safe libraries from the tttapa sysroot.
FROM debian:bookworm-slim

ARG RUST_TARGET=x86_64-unknown-linux-gnu

LABEL org.opencontainers.image.title="wyoming-satellite" \
      org.opencontainers.image.description="Wyoming protocol satellite for Home Assistant voice pipelines" \
      org.opencontainers.image.source="https://github.com/bryanboettcher/wyoming-rust" \
      org.opencontainers.image.licenses="GPL-3.0-only" \
      io.wyoming.target="${RUST_TARGET}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libasound2 \
        wget \
    && rm -rf /var/lib/apt/lists/*

# Overlay ARMv6 sysroot libraries on top of Debian's arm/v7 libs.
# For non-ARMv6 builds, /runtime-libs/ is empty so this is a no-op.
# For ARMv6 builds, the overlay also includes an empty /etc/ld.so.cache to
# prevent the dynamic linker from using cached paths to the old arm/v7 .so files.
# (We can't use a RUN to delete it because the glibc downgrade from 2.36 to 2.31
# breaks the container's own /bin/sh.)
COPY --from=builder /runtime-libs/ /

# ALSA config path fix: alsa-lib was cross-compiled with --prefix pointing to
# the tttapa sysroot, so its compiled-in config dir doesn't exist in the
# runtime container. Point ALSA to the Debian-provided config files instead.
ENV ALSA_CONFIG_DIR=/usr/share/alsa

# Tell the dynamic linker where to find shared libraries in Debian's multiarch
# directory. The tttapa sysroot's ld-linux-armhf.so.3 only searches /lib and
# /usr/lib by default, and we emptied ld.so.cache (can't regenerate it after
# the glibc downgrade). Without this, ARMv6 builds fail with "cannot open
# shared object file" for libs in /usr/lib/arm-linux-gnueabihf/.
# Harmless on non-ARMv6 builds (the cache still works, this just adds a
# redundant search path).
ENV LD_LIBRARY_PATH=/usr/lib/arm-linux-gnueabihf

COPY --from=builder /output/wyoming-satellite /usr/local/bin/wyoming-satellite

EXPOSE 10700 8585

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -q --spider http://localhost:8585/health || exit 1

ENTRYPOINT ["wyoming-satellite"]
