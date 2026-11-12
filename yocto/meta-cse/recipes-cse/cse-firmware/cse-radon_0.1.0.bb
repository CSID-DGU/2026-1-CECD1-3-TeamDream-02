CSE_BOARD = "radon"
# this board's port on the tunnel server (server/tunnel/sshd_config PermitListen, authorized_keys permitlisten)
CSE_TUNNEL_RPORT = "22041"
SRC_URI[bin.sha256sum] = "73b7cbb04ee3179c88beef4fbb4ffc206f79c2946998e2efdb8db433911a29d3"
require cse-firmware.inc
