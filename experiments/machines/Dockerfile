FROM herdrpp-navigator-base:latest
RUN printf '%s\n' 'deb https://deb.debian.org/debian bookworm main' > /etc/apt/sources.list \
    && apt-get update && apt-get install -y --no-install-recommends openssh-server openssh-client \
    && mkdir -p /run/sshd && rm -rf /var/lib/apt/lists/*
RUN /opt/snapshots/bin/pip install --no-cache-dir pytest==9.1.1
WORKDIR /workspace
