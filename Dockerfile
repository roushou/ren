# Ren daemon image, for Docker and Podman alike.
#
#   docker build -t ren .        podman build -t ren .
#
# The image runs `ren serve` as a non-root user with its data in the /data
# volume, under /data/ren, which Ren creates owner-only. The
# listener stays on loopback, as everywhere else: reach it from a Tailscale
# container that shares its network namespace (deploy/container), never from a
# published port. The UI is not included.

FROM docker.io/library/rust:1.99.0-slim-trixie AS build
RUN apt-get update \
  && apt-get install -y --no-install-recommends git \
  && rm -rf /var/lib/apt/lists/*
# The toolchain the image ships is the one rust-toolchain.toml pins; overriding
# the file skips its editor components.
ENV RUSTUP_TOOLCHAIN=1.99.0 CARGO_TERM_COLOR=never
WORKDIR /src
# The whole checkout, .git included, so build.rs records the source revision
# (`--version`, /health, doctor) exactly as a host build would. Build from a
# clone: a git worktree's .git points outside the context, so its revision
# reads `unknown`.
COPY . .
# No BuildKit cache mounts: Docker's legacy builder must build this too.
RUN cargo build --release --locked --package ren-rs --bin ren \
  && install -D target/release/ren /out/usr/local/bin/ren \
  && install -d /out/data

# glibc, libgcc, and CA certificates; no shell or package manager.
FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /out/usr/local/bin/ren /usr/local/bin/ren
COPY --from=build --chown=65532:65532 /out/data /data
VOLUME ["/data"]
USER 65532:65532
# The default data directory becomes /data/ren, for `serve` and for commands
# run with `exec` alike. Ren creates it owner-only, whatever mode the
# container engine gives the volume's root.
ENV XDG_STATE_HOME=/data
ENTRYPOINT ["/usr/local/bin/ren"]
CMD ["serve"]
# Shutdown gives active requests 10 seconds, then drains capture: allow 30.
STOPSIGNAL SIGTERM
