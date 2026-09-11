FROM rust:1.85

# Install MinGW cross-toolchain and zip
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y mingw-w64 zip \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/ls_web

# Copy manifest first to leverage Docker cache for dependencies
COPY Cargo.toml Cargo.lock ./
# Provide a dummy main to allow cargo fetch to populate registry cache
RUN mkdir -p src && echo "fn main() { println!(\"dummy\"); }" > src/main.rs && \
    cargo fetch

# Copy the full source
COPY . .

# Ensure the Windows target is available (rustup may already be present in the image)
RUN rustup target add x86_64-pc-windows-gnu || true

# Build release for Windows x86_64 (GNU)
RUN cargo build --release --target x86_64-pc-windows-gnu

# Export built artifact to /out
RUN mkdir -p /out && \
    if [ -f target/x86_64-pc-windows-gnu/release/ls_web.exe ]; then \
      cp target/x86_64-pc-windows-gnu/release/ls_web.exe /out/; \
    elif [ -f target/x86_64-pc-windows-gnu/release/ls_web ]; then \
      cp target/x86_64-pc-windows-gnu/release/ls_web /out/; \
    fi

CMD ["true"]
