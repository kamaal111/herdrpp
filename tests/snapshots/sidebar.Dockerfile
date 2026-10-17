FROM rust:slim-bookworm@sha256:2c3a22f0a5533ea2dd5a16627bc841228151faa2d4de2644ac9987e4a2f1f2fa

# Freeze system libraries, including FreeType, as well as Rust.
COPY --from=ghcr.io/astral-sh/uv:0.13.1@sha256:4a1f3666816dafadd20ceb71eac9a7d56d7ef30279f7d64048519ab901ada336 /uv /usr/local/bin/uv
RUN rm -f /etc/apt/sources.list.d/* && printf '%s\n' \
    'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20261001T000000Z bookworm main' \
    > /etc/apt/sources.list \
    && apt-get update && apt-get install -y --no-install-recommends \
    git curl ca-certificates libfontconfig1 libfreetype6 \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://github.com/herdrdev/herdr/releases/download/v0.9.5/herdr-linux-x86_64 \
    -o /usr/local/bin/herdr \
    && echo '14d9e8bc8b4a9e0118d77c32266f76d461f09b40342eb23752960bd89f59929d  /usr/local/bin/herdr' | sha256sum -c - \
    && chmod +x /usr/local/bin/herdr
ENV UV_PROJECT_ENVIRONMENT=/opt/snapshots UV_PYTHON_INSTALL_DIR=/opt/python UV_PYTHON=3.15.0
COPY pyproject.toml uv.lock .python-version /opt/test-project/
RUN cd /opt/test-project && uv sync --locked
ENV RUSTUP_TOOLCHAIN=1.99.0
ENV HERDR_SNAPSHOT_CONTAINER=1
ENV PYTEST_ADDOPTS="-o cache_dir=/output/pytest-cache"
ENV PATH="/opt/snapshots/bin:${PATH}" CARGO_TARGET_DIR=/output/build CARGO_HOME=/output/cargo
WORKDIR /workspace
CMD ["uv", "run", "--locked", "pytest", "-m", "snapshot", "tests/test_sidebar_snapshots.py"]
