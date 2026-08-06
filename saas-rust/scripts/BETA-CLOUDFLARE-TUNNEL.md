# Local Beta Cloudflare Quick Tunnel

The script saas-rust/scripts/beta-cloudflare-tunnel.sh is an isolated wrapper
for the Rust Beta service. It proxies only http://127.0.0.1:18090 by default
and never starts the legacy Python service or kills unrelated processes. It
uses an anonymous temporary Quick Tunnel; no Cloudflare token or credential
file is read.

Start the local Rust app first, then run:

    cd /home/dev/tgbot-rust-beta/saas-rust
    ./scripts/beta-cloudflare-tunnel.sh --dry-run start
    ./scripts/beta-cloudflare-tunnel.sh start
    ./scripts/beta-cloudflare-tunnel.sh status
    ./scripts/beta-cloudflare-tunnel.sh probe
    ./scripts/beta-cloudflare-tunnel.sh stop

Optional overrides are BETA_CF_ORIGIN_PORT, BETA_CF_BIN, BETA_CF_STATE_DIR,
and BETA_CF_WAIT_SECONDS. BETA_CF_ORIGIN_HOST is deliberately rejected
unless it is 127.0.0.1. The wrapper records only its own PID and log under
saas-rust/target/beta-cloudflare (ignored build output). The cloudflared process is detached from the launching WSL shell so a later status/probe command can verify the same tunnel.

