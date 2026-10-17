FROM herdrpp-machine-base:latest
RUN apt-get update && apt-get install -y --no-install-recommends openssh-server openssh-client \
    && mkdir -p /run/sshd && rm -rf /var/lib/apt/lists/*
RUN /opt/snapshots/bin/pip install --no-cache-dir pytest==9.1.1
WORKDIR /workspace
