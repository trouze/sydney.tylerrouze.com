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

**4. Add GitHub Actions secrets** (`Settings → Secrets and variables → Actions`):

| Secret | Value |
|--------|-------|
| `DEPLOY_SSH_KEY` | Contents of `~/.ssh/wedding_deploy` |
| `VM_NAME` | Your VM name (e.g. `sydney-tylerrouze-com`) |

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
