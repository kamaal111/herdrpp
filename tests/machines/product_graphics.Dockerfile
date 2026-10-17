# Build herdrpp-machine-base using tests/snapshots/picker.Dockerfile first.
FROM herdrpp-machine-base:latest
RUN apt-get update && apt-get install -y --no-install-recommends \
    kitty xvfb imagemagick xauth xdotool libgl1-mesa-dri fonts-dejavu-core openssh-server openssh-client \
    && mkdir -p /run/sshd && rm -rf /var/lib/apt/lists/*
ENV LIBGL_ALWAYS_SOFTWARE=1
