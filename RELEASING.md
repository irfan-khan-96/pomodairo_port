# Releasing Pomodairo

GitHub Actions (`.github/workflows/build.yml`) builds Windows, macOS and Debian/Ubuntu
installers on every push and pull request and attaches them to the run. A tag push
also creates a draft GitHub release with signed update packages and `latest.json`.

## Steps

1. Bump `version` in `app/src-tauri/tauri.conf.json`, `app/src-tauri/Cargo.toml` and
   `app/package.json` (then `cargo update -p pomodairo` in `app/src-tauri` and
   `npm install --package-lock-only` in `app/` to refresh the lock files).
2. Commit, push, and wait for CI to pass.
3. Tag and push: `git tag v0.3.0` and `git push origin v0.3.0`.
4. When the tag build finishes, review the draft release on GitHub and **publish** it.
   Installed copies only see a release once it is published.

## Update signing

Installed copies check `latest.json` on the latest published release (shortly after
start, then every 6 hours) and only install updates signed with the project's key.
The public key is in `app/src-tauri/tauri.conf.json`. The private key is kept outside
the repository and given to CI as repository secrets:

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the private signing key file |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Its password; leave the secret out if the key has none |

Keep a backup of the private key: without it, installed copies can't be updated and
everyone has to reinstall by hand.

Local builds (`npm run tauri build`) don't need the key. To test a signed build locally,
set both variables and add `--config src-tauri/tauri.release.conf.json`.

The `.deb` has no automatic updates; Debian/Ubuntu users install the new `.deb` by hand.
