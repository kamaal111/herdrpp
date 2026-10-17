FROM rust:slim-bookworm@sha256:2c3a22f0a5533ea2dd5a16627bc841228151faa2d4de2644ac9987e4a2f1f2fa

# Freeze system libraries, including Python and FreeType, as well as Rust.
RUN rm -f /etc/apt/sources.list.d/* && printf '%s\n' \
    'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20261001T000000Z bookworm main' \
    > /etc/apt/sources.list \
    && apt-get update && apt-get install -y --no-install-recommends \
    python3 python3-venv git curl ca-certificates libfontconfig1 libfreetype6 \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://github.com/herdrdev/herdr/releases/download/v0.9.5/herdr-linux-x86_64 \
    -o /usr/local/bin/herdr \
    && echo '14d9e8bc8b4a9e0118d77c32266f76d461f09b40342eb23752960bd89f59929d  /usr/local/bin/herdr' | sha256sum -c - \
    && chmod +x /usr/local/bin/herdr
COPY tests/snapshots/requirements.txt /tmp/requirements.txt
RUN python3 -m venv /opt/snapshots \
    && /opt/snapshots/bin/pip install --no-cache-dir -r /tmp/requirements.txt
ENV PATH="/opt/snapshots/bin:${PATH}" CARGO_TARGET_DIR=/output/build CARGO_HOME=/output/cargo
WORKDIR /workspace
CMD ["python3", "tests/picker_snapshots.py"]
