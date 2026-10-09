# The chummer-rs relay

## In short: what is a relay?

When a GM hosts an online campaign, the GM's own copy of chummer-rs is the
"server": players' apps connect straight to it. Home internet connections
usually sit behind a router, which makes a direct connection hard, so the
apps first ask a **relay** for help:

- **Finding each other.** Both apps check in with the relay. It tells them
  how to reach each other, and most of the time they then talk directly,
  without the relay in between.
- **When a direct line is impossible** (some mobile, campus or office
  networks), the relay passes the traffic along. It is encrypted end to
  end, so the relay cannot read it.
- **The mailbox, for play-by-post.** If the GM's app is closed, a player's
  changes wait in the relay's mailbox, sealed so that only the GM can open
  them, and the GM's app picks them up the next time it runs. Changes for
  offline players wait the same way.

You need no port forwarding, VPN or account. The relay stores no personal
data: only sealed messages (deleted once collected, or after 30 days) and
the public keys allowed to post into each mailbox.

**Which relay does the app use?** The project's public relay,
`chummerrs-default-relay.shambla.com`, is the default. You can run your own
relay instead (on a home server, a small VPS or a Raspberry Pi) and enter it
in the app under Tools → Online Settings → Relays. Everyone in a campaign
should use the same relay. The rest of this page explains how to host one;
the quickest way is [Deploy with Docker](#deploy-with-docker).

## Details

`chummer-relay` is one small binary with two jobs:

1. **Relay.** An [iroh relay server](https://docs.rs/iroh-relay) (iroh-relay
   1.3). It helps peers find a path to each other (hole punching) and
   forwards their traffic when no direct path exists. That traffic is
   end-to-end encrypted QUIC: the relay cannot read it.
2. **Mailbox.** An iroh endpoint beside the relay that serves the
   `chummer-rs/mailbox/3` protocol. It keeps sealed messages for peers who
   are offline (play-by-post, a GM who is not online). The messages are
   encrypted to the recipient's key and signed by the sender before they
   leave the app, so the operator cannot read them or see who wrote them.
   A mailbox only takes mail signed by keys its owner registered (see
   [Who may put mail](#who-may-put-mail)).

The relay keeps no accounts and no personal data. It stores only sealed
blobs, keyed by the recipient's node id, until they are collected or
expire, and for each mailbox the public keys that may put mail into it.

The project's public relay is the default in the app
(`chummer_net::config::DEFAULT_RELAY_URL`,
`https://chummerrs-default-relay.shambla.com`). Anyone can run their own one and
point their app at it. How GMs host and players join is in the README's
[Online campaigns](../README.md#online-campaigns); the design is in
[online-design.md](online-design.md).

The easiest way to run it is the Docker image
`ghcr.io/shamblashini/chummer-relay` (amd64 and arm64, see
[Deploy with Docker](#deploy-with-docker)). The release's Linux package
also includes `chummer-relay` (and the files of `packaging/relay/`); on
other systems build it with `cargo build --release -p chummer-relay`.

## What you need

- A server with a public IPv4 address (IPv6 too, if you have it).
- A DNS name for it, for example `relay.example.org`: an `A` record (and
  `AAAA` for IPv6) that points at the server.
- These ports open in the firewall:

  | Port | Protocol | Used for |
  |---|---|---|
  | 443 | TCP | The relay (HTTPS/WebSocket), and Let's Encrypt TLS-ALPN-01 challenges |
  | 80 | TCP | Captive-portal probe (iroh-relay always serves it on plain HTTP) |
  | 7842 | UDP | QUIC address discovery: tells clients their public address, for hole punching |
  | 7843 | UDP | Direct connections to the mailbox node (optional: without it, mail goes through the relay) |

  Metrics (`metrics_bind`, off by default) should stay on `127.0.0.1`.

- A certificate. The default is Let's Encrypt: the relay gets and renews
  it by itself over port 443 (TLS-ALPN-01). It needs the DNS name to point at
  the server first; a contact email is optional. Certificates are cached in
  `<data_dir>/acme`. You can instead give it a certificate you already
  have (`cert_mode = "manual"`, PEM files, e.g. from certbot; restart the
  relay after renewal). Behind a reverse proxy that does HTTPS itself
  (Coolify, Caddy, nginx, Cloudflare), use `cert_mode = "proxy"`: the
  relay then needs no certificate (see [Deploy on Coolify](#deploy-on-coolify)).

A small VPS is enough. The mailbox database is bounded by the limits
below (per recipient: 1000 messages of at most 256 KiB, deleted after 30
days; at most 64 registration scopes of 256 keys per mailbox). Most traffic is the hole-punching handshake; traffic is relayed in
full only when two peers cannot connect directly.

## Configuration

Copy [`packaging/relay/relay.example.toml`](../packaging/relay/relay.example.toml)
to `/etc/chummer-relay/relay.toml` and set at least `hostname`
(`tls.contact_email` is optional). Every other key has a default (shown in the example).
`chummer-relay --print-config` prints the defaults.

Command-line flags override the file: `--config`, `--hostname`,
`--data-dir`, `--cert-mode lets-encrypt|manual|self-signed|proxy`,
`--contact-email`. So do the environment variables `RELAY_HOSTNAME`,
`RELAY_DATA_DIR`, `RELAY_CERT_MODE` and `RELAY_CONTACT_EMAIL` (empty ones
are ignored). `--dev` starts a local test relay (self-signed
certificate for `127.0.0.1`, HTTP 3340, HTTPS 3443).

### Mailbox limits

| Key | Default | Meaning |
|---|---|---|
| `max_blob_bytes` | 262144 (256 KiB) | Largest sealed message |
| `max_messages_per_recipient` | 1000 | Messages waiting for one recipient |
| `max_messages_per_sender_per_day` | 2000 | Uploads per sender per UTC day |
| `max_bytes_per_sender_per_day` | 67108864 (64 MiB) | Upload bytes per sender per UTC day |
| `max_messages_per_key` | 200 | Messages waiting in one mailbox that were signed by one key |
| `max_messages_per_unbound_key` | 10 | The same, for a key registered without an uploading node (an invite nobody claimed yet) |
| `expiry_secs` | 2592000 (30 days) | Uncollected messages are deleted after this |

Senders and recipients are identified by the node id that the QUIC
handshake proves. A connection can only fetch, delete and configure its
own mailbox. Expired mail is purged every hour.

### Who may put mail

Access is by capability; there are no accounts.

- **Registration.** A mailbox owner sends `Register { scope, keys }`: the
  public keys that may put mail into its mailbox, for one scope (the
  apps use one scope per campaign), each optionally bound to one
  uploading node. It replaces that scope's list; the same list again
  changes nothing; an empty list removes the scope. A key in several
  scopes may be uploaded by any node that one of them names, or by any
  node if one of them leaves it unbound. The registrations are in
  `mailbox.redb` and survive restarts. The GM's app registers every
  current invite key of the campaign (and the node keys of players it
  added by node id) at every mailbox round, right after an invite changed
  and right after a player claimed an invite; each player's app registers
  the GM's campaign keys, bound to the GM's node.
- **Bound keys.** A claimed invite's key is bound to the node that
  claimed it, and a member added by node id to that node. A put signed
  with a bound key but uploaded by another node is refused ("this key may
  only put mail from another device"). So a link that leaks after it was
  claimed is useless for mail: the leaked key cannot put a single
  message. An unclaimed invite's key is unbound (its player may join by
  mail from any device), and at most `max_messages_per_unbound_key`
  messages signed by it may wait.
- **Signed puts.** Every put carries the signing key, a random nonce and
  an ed25519 signature over the recipient's node id, the uploading
  node's id, the nonce and the blob's BLAKE3 hash. The relay refuses an
  unsigned put, a bad signature, a key the recipient did not register,
  a nonce still waiting in that mailbox (a replay), and any put to a
  mailbox with no registrations (there is no open mode). A put signed
  for one mailbox cannot be replayed into another, or by another node.
- **Caps.** The per-sender daily quotas stay. As a second net, at most
  `max_messages_per_key` messages signed by one key may wait in one
  mailbox (for bound keys this counts only the key holder's own mail).
- **Revocation** is the owner registering the list without the key: from
  then on the key's puts are refused. Mail already waiting stays until
  the owner collects it (the GM's app drops it).
- The answer to a registration is the mailbox's state: keys registered,
  messages waiting (per key) and puts refused today. The GM screen shows
  it. Refused puts are counted in memory only, for mailboxes with
  registrations.

A database of an older layout (the first protocol's, or `mailbox/2`'s
without bound keys) is emptied when the relay opens it: it only held
mail in transit and registrations, which the apps send again at their
next mailbox round; apps mail again what was not answered. Apps and
relays must both speak `chummer-rs/mailbox/3`.

`relay_rate_limit` (bytes per second per client, off by default) limits
relayed traffic.

## Deploy with Docker

Every release publishes the image `ghcr.io/shamblashini/chummer-relay`
with two tags: the version (`0.5.0`) and `latest`. Between releases, the
"Relay image" workflow publishes `:edge` (and `:sha-<commit>`) whenever
the relay or the code it is built from changes on master; it can also be
started by hand (Actions → Relay image → Run workflow) with a tag of your
choice. To follow it, set `image: ghcr.io/shamblashini/chummer-relay:edge`
in `docker-compose.yml`; the relay and the apps must speak the same
protocol, so use `edge` only with app builds from master. Each release also
carries `docker-compose.yml` and `relay.example.toml`. On the server:

```bash
mkdir chummer-relay && cd chummer-relay
base=https://github.com/shamblashini/chummer-rs/releases/latest/download
curl -LO $base/docker-compose.yml -LO $base/relay.example.toml
cp relay.example.toml relay.toml     # edit hostname and contact_email
docker compose up -d
docker compose logs chummer-relay    # shows the mailbox node id
```

To update: `docker compose pull && docker compose up -d`. To stay on one
version, change `:latest` in `docker-compose.yml` to the version you
want.

Without Compose:

```bash
docker run -d --name chummer-relay --restart unless-stopped --network host \
  -v "$PWD/relay.toml:/etc/chummer-relay/relay.toml:ro" \
  -v chummer-relay-data:/var/lib/chummer-relay \
  -e RUST_LOG=info,iroh=warn \
  ghcr.io/shamblashini/chummer-relay:latest
```

The compose file uses host networking, so the relay sees clients' real
addresses (QUIC address discovery reports them back) and IPv6 works. The
relay listens on TCP 80 and 443 and UDP 7842 and 7843; open them in the
server's firewall. The data directory is the `chummer-relay-data` volume;
back it up (see below). The container runs as the unprivileged user
`chummer-relay`.

To build the image from a source checkout instead:

```bash
git clone https://github.com/shamblashini/chummer-rs
cd chummer-rs/packaging/relay
cp relay.example.toml relay.toml     # edit hostname and contact_email
docker compose -f docker-compose.yml -f docker-compose.build.yml up -d --build
```

## Deploy on Coolify

[Coolify](https://coolify.io) runs its own proxy (Traefik) on TCP 80 and
443, so the plain compose file (host networking, its own certificate)
clashes with it. Use `packaging/relay/docker-compose.coolify.yml` instead
(also attached to every release). The relay then runs with
`cert_mode = "proxy"`:

- Coolify's proxy does HTTPS for the relay's domain, with the certificate
  Coolify gets from Let's Encrypt, and forwards plain HTTP (websockets
  included) to the relay's port 80.
- The relay needs no certificate, no email and no open ports on the
  server.
- QUIC address discovery is off, because it needs the relay's own
  certificate and UDP. Peers still connect through the relay, which is all
  play-by-post needs. Live sessions find a direct path less often, so
  more traffic goes through the relay (and Cloudflare, if used): a little
  more latency. For the most direct connections, use a DNS-only
  (grey-cloud) name and the host-networking `docker-compose.yml` outside
  Coolify's proxy instead.

The same works with the domain behind Cloudflare's proxy (orange cloud).
Cloudflare carries the HTTPS and websocket traffic, and nothing else is
needed. Use SSL mode "Full (strict)", since Coolify's certificate is a
real one.

Steps:

1. Point a DNS name at the Coolify server, e.g. `relay.example.org`
   (proxied through Cloudflare or not).
2. In Coolify: **New resource → Docker Compose Empty**, and paste
   `docker-compose.coolify.yml`.
3. Set the `chummer-relay` service's **Domain** to
   `https://relay.example.org`. Coolify routes it to the container's
   port 80 and gets the certificate.
4. Optional, under **Environment Variables**: `RELAY_HOSTNAME`
   (`relay.example.org`) so the log prints the complete relay entry, and
   `RELAY_TAG` (`edge` by default, which follows master; `latest` or a
   version once a release has proxy mode).
5. Deploy. The logs show `mailbox node connected to the relay` and the
   mailbox node id. The relay entry for the app is
   `https://relay.example.org#<mailbox node id>`; see
   [The mailbox node id](#the-mailbox-node-id).

The data (the mailbox, and the mailbox node key) lives in the
`chummer-relay-data` volume. Back it up: a new key means a new mailbox
node id, and every app would need the new entry.

To check it from your machine:

```bash
curl -s -o /dev/null -w "%{http_code}\n" https://relay.example.org/ping
```

`200` means the proxy reaches the relay.

Any other TLS-terminating reverse proxy (Caddy, nginx) works the same
way. Set `cert_mode = "proxy"` in `relay.toml`, and forward the domain,
websocket upgrades included, to the relay's `http_bind`.

## Deploy with systemd

```bash
cargo build --release -p chummer-relay
sudo install -m755 target/release/chummer-relay /usr/local/bin/
sudo install -Dm644 packaging/relay/relay.example.toml /etc/chummer-relay/relay.toml
sudoedit /etc/chummer-relay/relay.toml    # hostname, contact_email
sudo install -m644 packaging/relay/chummer-relay.service /etc/systemd/system/
sudo systemctl enable --now chummer-relay
journalctl -u chummer-relay               # shows the mailbox node id
```

The unit runs as a dynamic user with only `CAP_NET_BIND_SERVICE` and keeps
its data in `/var/lib/chummer-relay`. With `cert_mode = "manual"`, that
user cannot read root-only files such as `/etc/letsencrypt/live/.../privkey.pem`:
copy the certificate and key into a readable place in a certbot deploy hook,
or use Let's Encrypt mode instead.

The mailbox node connects to the relay through `hostname`, so the server
must be able to reach its own public name. The log says "mailbox node
connected to the relay" when it works.

## The mailbox node id

On start-up the relay logs two lines like:

```text
mailbox node id: a14cbbf284e289ed5ba9475d0456881cb34b58006174e7376b6a879cfb79a7b4
give users this relay entry: https://relay.example.org/#a14cbbf2...a7b4
```

The mailbox is an iroh endpoint, so clients dial it by its node id. The id
comes from `<data_dir>/mailbox.key`, which is made on the first start.
**Keep this file and back it up.** If it is lost, the relay gets a new id
and every app that knows the old one can no longer reach the mailbox
(mail already stored is lost too, as it is in `mailbox.redb` in the same
directory).

The relay entry (`<url>#<mailbox-id>`) is what an app needs to use this
relay and its mailbox.

## Point the app at a relay

The app's network settings are a list of relay entries
(`chummer_net::config::NetConfig`). The default list is the project's
public relay. In the app, Tools → Online Settings → Relays takes one entry
per line (`https://relay.example.org#<mailbox node id>`, as the relay
prints it at start-up); "Project relay" puts the default back. The
settings are stored in `online.json` in the chummer-rs config folder,
which `chummer-authority` reads too (or give `--relay`). The app uses the
relay with the lowest latency as its home relay and looks peers up on
all of them; the first entry with a mailbox is used for play-by-post.
Changed relays take effect after the app restarts.

A GM on a private relay can put it into invite links
(`&relay=<url>`), so players who do not have it in their list can still
connect.

For a `self-signed` relay, clients must also trust its certificate
(`<data_dir>/self-signed-cert.pem`, `NetConfig::extra_ca_roots`). Use this
only for testing or a private group; with a public name, use Let's
Encrypt.

## Local test relay

`chummer-relay --dev` runs a relay for tests on this machine: hostname
`127.0.0.1`, a self-signed certificate made in the data directory, HTTP
on 3340, HTTPS on 3443, QUIC address discovery on 7842 and the mailbox
on 7843.

```bash
mkdir relay-dev && cd relay-dev
chummer-relay --dev --data-dir .
# prints: give users this relay entry: https://127.0.0.1:3443/#<mailbox id>
```

In each app (give each one its own `XDG_CONFIG_HOME`, so each has its
own node key), open Tools → Online Settings, enter that relay entry and
add `relay-dev/self-signed-cert.pem` under Trusted certificates. For
`chummer-authority`, pass `--relay <entry> --ca relay-dev/self-signed-cert.pem`.

## How peers find each other

There is no directory of users. A player dials the GM by the node id in
the invite link. The app tries that id on every relay in its list (and the
link's relay hint); a relay forwards packets to any endpoint connected to
it, so the first packets go through the GM's home relay and iroh then
punches a direct path. iroh's own lookup services (pkarr and DNS on
number 0's `dns.iroh.link`) are not used, so no third-party servers are
involved.

## Backups and privacy

- Back up `<data_dir>/mailbox.key` (the mailbox identity). `mailbox.redb`
  holds only sealed, expiring messages; losing it loses undelivered mail.
  The apps mail again what was not answered after a day
  (`PlayerConfig::remail_after`), so lost mail is sent again.
- If `mailbox.redb` is damaged, the relay moves it to
  `mailbox.redb.damaged-<unix time>`, logs an error and starts with an
  empty one. It does not stop. A file it cannot read (permissions) is an
  error at start-up.
- The relay logs connection events at `info` level without message
  contents. It never sees plaintext: campaign traffic is end-to-end
  encrypted QUIC, and mailbox blobs are sealed boxes (X25519 +
  XSalsa20-Poly1305) to the recipient, signed (ed25519) by the sender.
- The relay can see which node ids are connected, and for the mailbox,
  which id sent how many bytes to which id, and when, and which public
  keys each mailbox takes mail from (random per invite; they name no
  one).
