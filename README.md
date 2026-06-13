# sydney.tylerrouze.com
We're getting married at Schrute farms no matter what

## Deployment

Releases deploy automatically to the exe.dev VM when a tag is pushed:

```bash
git tag v1.0.0 && git push origin v1.0.0
```

The GitHub Action builds the binary on Ubuntu, copies it to the VM via `scp`, and restarts the systemd service.

### First-time setup

**1. Generate a deploy SSH key (no passphrase):**
```bash
ssh-keygen -t ed25519 -C "github-deploy" -f ~/.ssh/wedding_deploy -N ""
```

**2. Add the public key to your exe.dev account:**
```bash
cat ~/.ssh/wedding_deploy.pub | ssh exe.dev ssh-key add
```

**3. Find your VM name:**
```bash
ssh exe.dev ls --json | jq -r '.vms[0].vm_name'
# e.g. "sydney-tylerrouze-com" → VM SSH host is sydney-tylerrouze-com.exe.xyz
```

**4. Add GitHub Actions secrets and variables** (`Settings → Secrets and variables → Actions`):

App config is the source of truth in GitHub: each deploy rewrites
`/home/exedev/sydney.tylerrouze.com/wedding.env` on the VM from these, and the
systemd unit reads it. A value left unset is simply omitted (the app uses its
default; the listmonk sync stays off unless URL+USER+TOKEN are all present).

| Secret | Value |
|--------|-------|
| `DEPLOY_SSH_KEY` | Contents of `~/.ssh/wedding_deploy` |
| `VM_NAME` | Your VM name (e.g. `sydney-tylerrouze-com`) |
| `ADMIN_TOKEN` | Admin dashboard password |
| `LISTMONK_TOKEN` | listmonk API access token |

| Variable | Value |
|----------|-------|
| `LISTMONK_URL` | e.g. `https://mailing.tylerrouze.com` |
| `LISTMONK_USER` | listmonk API username |
| `LISTMONK_LIST_ID` | Target list id (defaults to `4` in-app) |
| `DATABASE_URL` | Optional; defaults to `sqlite:data/wedding.db` |
| `RESET_DB` | `true` to wipe the DB pre-launch (see deploy.yml) |

> Because the deploy recreates `wedding.env`, anything you previously set by hand
> on the VM (e.g. `ADMIN_TOKEN`) must be added here, or it'll be dropped on the
> next deploy.

Set the app config from the template files (each `*.example` lists every key —
copy, fill in real values, and push). The filled files are gitignored:

```bash
cp .env.secrets.example   .env.secrets      # ADMIN_TOKEN, LISTMONK_TOKEN
cp .env.variables.example .env.variables    # LISTMONK_URL, LISTMONK_USER, ...
# edit both, then:
gh secret set   -f .env.secrets
gh variable set -f .env.variables
```

`DEPLOY_SSH_KEY` (multi-line) and `VM_NAME` are one-offs, set them directly:

```bash
gh secret set DEPLOY_SSH_KEY < ~/.ssh/wedding_deploy
gh secret set VM_NAME --body "sydney-tylerrouze-com"
```

**5. Run the setup script on first VM boot:**
```bash
ssh exedev@<vm-name>.exe.xyz 'bash -s' < setup.sh
```

## Local dev

```bash
DATABASE_URL=sqlite:data/wedding.db RUST_LOG=debug cargo run
```
For local testing of the admin flow before any of this:
```
ADMIN_TOKEN=devtoken COOKIE_INSECURE=1 cargo run
```
# visit http://localhost:8080/admin/login, password: devtoken

## Mailing list (listmonk)

When a guest RSVPs and provides an email, we add them (best-effort, in the
background) to a listmonk list. New subscribers are created pre-confirmed; ones
that already exist are added to the list without disturbing their other
subscriptions. Configure via env vars — if any of URL/USER/TOKEN is unset the
integration is disabled (so dev and tests never call out):

| Var | Notes |
| --- | --- |
| `LISTMONK_URL` | Base URL, e.g. `https://mailing.tylerrouze.com` |
| `LISTMONK_USER` | API username |
| `LISTMONK_TOKEN` | API access token |
| `LISTMONK_LIST_ID` | Target list id (defaults to `4`) |
