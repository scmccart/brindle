FROM ubuntu:26.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        openssh-server \
    && useradd --create-home --shell /bin/bash remote \
    && mkdir -p /run/sshd /root/.ssh \
    && chmod 0700 /root/.ssh \
    && ssh-keygen -A \
    && passwd -d root \
    && rm -rf /var/lib/apt/lists/*

COPY sshd_config /etc/ssh/sshd_config.d/brindle-e2e.conf

CMD ["sleep", "infinity"]
