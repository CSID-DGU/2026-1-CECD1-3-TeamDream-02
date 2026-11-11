# sshd that accepts nothing but reverse tunnels from the boards (remote maintenance), kept apart from the host's own
# sshd. Build: docker build -t cse-tunnel:1 server/tunnel/
FROM docker.io/library/alpine:3.22
RUN apk add --no-cache openssh-server \
    && adduser -D -H -s /sbin/nologin tunnel \
    # "!" (locked) makes sshd refuse the account even with a key; "*" = no password matches, not locked
    && sed -i 's/^tunnel:!/tunnel:*/' /etc/shadow \
    && mkdir -p /etc/ssh/authorized_keys /data
COPY sshd_config /etc/ssh/sshd_config
COPY entrypoint.sh /entrypoint.sh
RUN chmod 755 /entrypoint.sh
VOLUME /data
EXPOSE 2222
ENTRYPOINT ["/entrypoint.sh"]
