# Android Release Delivery

Use the dedicated formal build and upload command:

```powershell
pnpm android:arm64:release:github
```

The command performs the following steps:

1. Builds a formally signed ARM64 release APK.
2. Verifies the signer, certificate pin, ABI, and debuggable state.
3. Copies the timestamped APK to `release-artifacts/`.
4. Uploads it to the existing `v<package-version>` GitHub Release.
5. Reads the uploaded asset back and verifies its size and SHA-256 digest.

The default destination is `Huanghgj/VCPMobile`. Override it only when needed:

```powershell
$env:VCPMOBILE_GITHUB_REPO = "owner/repository"
$env:VCPMOBILE_GITHUB_RELEASE_TAG = "v1.1.4"
pnpm android:arm64:release:github
```

Safety properties:

- The normal `pnpm android:arm64:release` command remains local-only.
- Debug-signed APKs cannot use the GitHub upload path.
- Existing assets are never overwritten.
- A same-name asset is accepted only when its size and SHA-256 match.
- The target Release must already exist; the script never creates a tag.
- Upload failures leave the verified local APK in `release-artifacts/`.

GitHub CLI must be authenticated before running the command:

```powershell
gh auth status
```
