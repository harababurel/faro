# getfaro.dev server runbook

How to set up the server for getfaro.dev and its self-hosted Plausible (M13 Phase 0; design in [docs/WEBSITE.md](../../docs/WEBSITE.md) §6.8–§6.10). Run it by hand, top to bottom.

It assumes a Debian/Ubuntu server with Docker (and the Compose plugin), nginx, certbot (`python3-certbot-nginx`), git and openssl. It also assumes a CPU that ClickHouse supports and at least 2 GB of free RAM.

| File | Goes to |
|---|---|
| [nginx/getfaro.dev.conf](nginx/getfaro.dev.conf) | `/etc/nginx/sites-available/getfaro.dev` (fill in `PLAUSIBLE_SCRIPT`, step 6) |
| [nginx/snippets/getfaro-headers.conf](nginx/snippets/getfaro-headers.conf) | `/etc/nginx/snippets/getfaro-headers.conf` |
| [nginx/plausible.harababurel.com.conf](nginx/plausible.harababurel.com.conf) | `/etc/nginx/sites-available/plausible.harababurel.com` |
| [logrotate/getfaro](logrotate/getfaro) | `/etc/logrotate.d/getfaro` |
| [plausible/compose.override.yml](plausible/compose.override.yml) | `/srv/plausible/compose.override.yml` |
| [plausible/write-env.sh](plausible/write-env.sh) | run once, to write `/srv/plausible/.env` |
| [update.sh](update.sh) | run from the clone for every deploy |

The nginx files are **copied**, not included from the repo, because certbot edits the files it finds. If one changes in the repo, copy it again and re-run certbot. Don't edit the installed copy by hand.

**Other layouts.** The paths above are Debian's. Adapt them as needed; the runbook was also followed on a server without `sites-available`/`sites-enabled`:
- **nginx:** if all sites live in one `nginx.conf`, make a folder (say `/etc/nginx/sites.d/`), put the two site files there, and add `include /etc/nginx/sites.d/*.conf;` inside `http {}`. Skip the `ln -s` steps. certbot edits the files in that folder as usual.
- **logrotate:** `create 0640 www-data adm` needs a user and group that exist. Use `root adm` (nginx's master process opens the logs as root). Check that the logrotate timer is enabled and that `/run/nginx.pid` exists.
- **Dynamic DNS:** if the IP changes, the DNS records for both domains have to be kept up to date by your dynamic-DNS client.

---

## 0. Checks

```sh
ss -ltn | grep -E ':(80|443|8000)\b'     # 80/443 should be nginx; 8000 must be free
grep -m1 -o -w 'sse4_2' /proc/cpuinfo    # x86: must print sse4_2 (ARM: NEON is standard)
free -h                                  # ≥ 2 GB available
docker compose version && nginx -v && certbot --version
```

If 8000 is taken, pick another port and change it everywhere it appears: `.env` (`HTTP_PORT`), `compose.override.yml`, and both nginx files.

## 1. DNS

| Name | Type | Value | TTL |
|---|---|---|---|
| `getfaro.dev` | A (and AAAA if you have IPv6) | the server's IP | ≤ 300 |
| `www.getfaro.dev` | CNAME | `getfaro.dev` | ≤ 300 |
| `plausible.harababurel.com` | A (or CNAME to the server's name) | the server's IP | ≤ 300 |

The apex can't be a CNAME. If the IP is dynamic, keep these updated with your dynamic-DNS client. Check:

```sh
dig +short getfaro.dev www.getfaro.dev plausible.harababurel.com
```

## 2. Plausible CE (pinned to v3.2.1)

```sh
sudo git clone -b v3.2.1 --single-branch https://github.com/plausible/community-edition /srv/plausible
sudo chown -R "$USER": /srv/plausible
cp <repo>/site/deploy/plausible/compose.override.yml /srv/plausible/
<repo>/site/deploy/plausible/write-env.sh /srv/plausible     # secrets; never overwrites
cd /srv/plausible && docker compose up -d
curl -sI http://127.0.0.1:8000 | head -1                     # 200 OK or 302 Found (a redirect to the login page); give it a minute
ss -ltn | grep 8000                                          # must show 127.0.0.1:8000, not 0.0.0.0
```

`<repo>` is any checkout of faro: your laptop via `scp`, or the server clone from step 5.

**Save `/srv/plausible/.env` in your password manager.** `TOTP_VAULT_KEY` encrypts the 2FA secrets, so losing it locks you out of TOTP.

## 3. nginx and TLS for the dashboard

```sh
sudo cp <repo>/site/deploy/nginx/plausible.harababurel.com.conf /etc/nginx/sites-available/plausible.harababurel.com
sudo ln -s /etc/nginx/sites-available/plausible.harababurel.com /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
sudo certbot --nginx -d plausible.harababurel.com
```

## 4. Your Plausible account

1. Open https://plausible.harababurel.com and register: the first account becomes the owner. Until you close registration (step 3), other people can only join by invitation (`invite_only`). If Plausible refuses your first registration, set `DISABLE_REGISTRATION=false` in `.env`, run `docker compose up -d`, register, then carry on with step 3.
2. Account settings → **Two-factor authentication**: turn it on and store the recovery codes.
3. Close registration:
   ```sh
   cd /srv/plausible && sed -i 's/^DISABLE_REGISTRATION=.*/DISABLE_REGISTRATION=true/' .env && docker compose up -d
   ```
4. **Add a website:** domain `getfaro.dev`. In the installation options, turn on **outbound links**.
5. From the snippet Plausible shows, **note the script name**: the `pa-XXXXXXXX` part of `…/js/pa-XXXXXXXX.js`. Ignore the rest of the snippet; the site already contains its equivalent (`site/placeholder/js/init.js`).
6. Site settings → **Goals** → add these custom events (WEBSITE.md §6.9; only "Install copied" fires before Phase 4): `Demo started`, `Demo focused`, `Demo message sent`, `Theme changed`, `Install copied`.

There's no email set up, so a lost password can't be reset by email. Keep the credentials in your password manager.

## 5. The repo and the first deploy

The site is built from a clone on the server, owned by your normal user (not root). While the GitHub repo is private, it's cloned with a read-only deploy key.

```sh
ssh-keygen -t ed25519 -N '' -C 'getfaro.dev deploy (read-only)' -f ~/.ssh/getfaro_deploy
cat ~/.ssh/getfaro_deploy.pub
```

Add the key on GitHub at harababurel/faro → Settings → Deploy keys → **Add deploy key**, with "Allow write access" **off**. Then:

```sh
cat >> ~/.ssh/config <<'EOF'
Host github-faro
    HostName github.com
    User git
    IdentityFile ~/.ssh/getfaro_deploy
    IdentitiesOnly yes
EOF
sudo install -d -o "$USER" -g "$USER" -m 755 /srv/getfaro.dev
git clone github-faro:harababurel/faro.git /srv/getfaro.dev/repo
/srv/getfaro.dev/repo/site/deploy/update.sh
ls -l /srv/getfaro.dev/current                 # -> releases/<commit>
```

The build is `site/build.sh`: it builds the page with `faro-site` and the web
demo's wasm module with `crates/faro-web/build.sh`, so it needs bash, git,
gzip **and** the Rust toolchain:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
# rust-toolchain.toml installs the pinned toolchain, with the wasm32 target:
rustup target add wasm32-unknown-unknown   # already there; harmless
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # must match Cargo.lock's wasm-bindgen
sudo apt install binaryen                 # optional: wasm-opt -Oz shrinks the module
sudo apt install brotli                   # optional: .br twins (nginx needs the brotli module to use them)
```

`crates/faro-web/build.sh` refuses to run if `wasm-bindgen --version` isn't the
version in `Cargo.lock`, so re-run the `cargo install` line whenever that
changes. To build only the page (no demo), run `SITE_NO_DEMO=1 site/build.sh`.

## 6. nginx and TLS for getfaro.dev

Replace `pa-XXXXXXXX` below with the script name from step 4.5:

```sh
sudo install -d -o www-data -g adm -m 755 /var/log/nginx/getfaro
sudo cp <repo>/site/deploy/nginx/snippets/getfaro-headers.conf /etc/nginx/snippets/
sed 's/PLAUSIBLE_SCRIPT/pa-XXXXXXXX/' <repo>/site/deploy/nginx/getfaro.dev.conf | sudo tee /etc/nginx/sites-available/getfaro.dev >/dev/null
sudo ln -s /etc/nginx/sites-available/getfaro.dev /etc/nginx/sites-enabled/
grep -n wasm /etc/nginx/mime.types             # must map application/wasm to wasm
sudo nginx -t && sudo systemctl reload nginx
sudo certbot --nginx -d getfaro.dev -d www.getfaro.dev
```

If `mime.types` lacks wasm, add `application/wasm wasm;` to it. Browsers won't stream-compile wasm served as anything else.

## 7. Log retention (30 days, as the privacy policy says)

```sh
sudo cp <repo>/site/deploy/logrotate/getfaro /etc/logrotate.d/getfaro
sudo logrotate -d /etc/logrotate.d/getfaro     # dry run: no errors, no "duplicate log entry"
```

The getfaro.dev logs are in their own folder (`/var/log/nginx/getfaro/`). Debian's `/etc/logrotate.d/nginx` already rotates `/var/log/nginx/*.log` after 14 days, and logrotate refuses two rules for the same file.

## 8. Check it end to end

```sh
curl -sI http://getfaro.dev | grep -iE '^(HTTP|location)'            # 301 -> https://getfaro.dev/
curl -sI https://www.getfaro.dev | grep -iE '^(HTTP|location)'       # 301 -> https://getfaro.dev/
curl -sI https://getfaro.dev | grep -iE '^(HTTP|content-security|strict-transport|referrer|x-content|permissions|cache-control)'
curl -sI https://getfaro.dev/js/s.js | grep -iE '^(HTTP|content-type)' # 200, javascript
curl -s -o /dev/null -w '%{http_code}\n' -X POST https://getfaro.dev/api/event \
  -H 'Content-Type: application/json' -H 'User-Agent: Mozilla/5.0 (X11; Linux x86_64) runbook-check' \
  -d '{"name":"pageview","url":"https://getfaro.dev/runbook-check","domain":"getfaro.dev"}'   # 202
```

Then open https://getfaro.dev in a browser that doesn't block it. Check that:
- the browser console shows no CSP errors;
- the dashboard's **realtime** view shows the visit, with your country;
- a test page view `/runbook-check` appears too.

If the country is missing or every visit has the server's location, the visitor IP isn't reaching Plausible: check the `X-Forwarded-For` lines in both nginx files.

Optional, from another machine: https://securityheaders.com/?q=getfaro.dev and the SSL Labs test.

## Updates

- **Site:** `/srv/getfaro.dev/repo/site/deploy/update.sh`. It pulls, builds into `releases/<commit>/`, switches `current` atomically and keeps the last five releases.
- **Rollback:**
  ```sh
  ln -sfn releases/<commit> /srv/getfaro.dev/.current-new && mv -T /srv/getfaro.dev/.current-new /srv/getfaro.dev/current
  ```
- **Plausible** (about twice a year): read the release notes. Then:
  ```sh
  cd /srv/plausible && git fetch --tags && git checkout <new tag> && docker compose pull && docker compose up -d
  ```
  Major versions may need extra steps.

## Backups

The statistics aren't critical. A weekly dump of the settings database is enough. For example, as a cron job:

```sh
cd /srv/plausible && docker compose exec -T plausible_db pg_dump -U postgres plausible_db | gzip > ~/backups/plausible-$(date +%F).sql.gz
```

ClickHouse's event data can be rebuilt by starting over, losing only history.
