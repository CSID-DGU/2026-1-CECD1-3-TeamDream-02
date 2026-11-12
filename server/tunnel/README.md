# Maintenance tunnel

The boards sit behind the LTE modem, so nobody can SSH into them from outside. Each board therefore keeps an
outgoing SSH connection to this container (`cse-tunnel.service` on the board) and asks it to forward one port back
to its own sshd. The container accepts nothing else: no shell, no file transfer, no other forwards, and each board
key can open only its own port.

```
board sshd :22  <--  board ssh -R  -->  pfSense WAN :22022  -->  this host :22022  -->  container sshd :2222
you:  ssh -J <this host> -p 22041 dreamteam@127.0.0.1      (radon; servo is 22042)
```

The forwarded ports are published on this host's `127.0.0.1` only, so the boards' sshd is reachable from this host
(or through it with `-J`), never from the internet. Logging in to a board still needs the board's own SSH key.

## Set up

1. pfSense: NAT port forward WAN tcp/22022 -> this host tcp/22022. HAProxy is not involved (SSH is not HTTP).
2. Here, in a directory with `compose.yaml`:
   ```
   docker build -t cse-tunnel:1 <repo>/server/tunnel/
   cp authorized_keys.example authorized_keys      # filled in step 4
   docker compose up -d && docker compose logs     # prints the host key fingerprint
   ```
3. On the PC, fetch the host key and compare its fingerprint with the one the container printed:
   ```
   ssh-keyscan -p 22022 radon.kafu.moe 2>/dev/null | grep ed25519 > tunnel_known_hosts
   ssh-keygen -lf tunnel_known_hosts
   ```
4. Give each board the destination and the pinned host key; the script prints the board's tunnel public key:
   ```
   TUNNEL_HOST=radon.kafu.moe TUNNEL_PORT=22022 TUNNEL_KNOWN_HOSTS=tunnel_known_hosts \
       yocto/apply-to-board.sh dreamteam@<board>
   ```
   Put the two public keys into `authorized_keys` here (format in `authorized_keys.example`), then
   `docker compose restart`. Reboot the boards (or `sudo systemctl restart cse-tunnel` on them).
5. Check: `docker compose logs` shows the two logins, and `ssh -J <this host> -p 22041 dreamteam@127.0.0.1` works.

## Notes

- `data/` holds the container's host key. The boards refuse to connect to any other key, so keep it (back it up);
  losing it means visiting the boards.
- A board whose connection died (modem restart, new LTE address) still holds its port for a while, and the board's
  new attempts are refused until the dead connection is closed (within about a minute: keepalive every 20 s,
  `tcp_retries2` in `compose.yaml`). The board retries every 60 s, so a tunnel is back 1-2 minutes after the
  board is online again.
- Removing a board's line from `authorized_keys` and restarting the container cuts that board off.
- Idle cost over LTE: one keepalive per minute, a few hundred kB per day.
