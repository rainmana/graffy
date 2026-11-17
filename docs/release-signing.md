# Signing and notarizing the macOS release

The release workflow (`.github/workflows/release.yml`) signs the macOS binary
with a **Developer ID Application** certificate and has Apple **notarize** it,
so downloads run without a Gatekeeper warning. It does this only when six
GitHub secrets exist. Without them the binary is built unsigned and the run
shows a warning that says so.

> **Status: written but never run.** These steps and the workflow step were
> written without access to an Apple account or a macOS runner. The commands
> follow Apple's and GitHub's documented approach, but the first real run is
> the test. Use the dry run in step 5 before tagging a release.

What you need: a paid Apple Developer Program membership, and a Mac with
Xcode or the Xcode command-line tools.

## 1. Create the Developer ID Application certificate

1. On your Mac, open **Keychain Access → Certificate Assistant → Request a
   Certificate From a Certificate Authority**. Enter your email, choose
   **Saved to disk**, and save the `.certSigningRequest` file.
2. At <https://developer.apple.com/account/resources/certificates>, click
   **+**, choose **Developer ID Application**, upload that request file, and
   download the resulting `.cer` file.
3. Double-click the `.cer` file to add it to your login keychain.

Only the account holder can create Developer ID certificates.

## 2. Export it as a `.p12` and note its name

1. In Keychain Access, under **My Certificates**, find
   **Developer ID Application: Your Name (TEAMID)**. Expand it; it must show a
   private key underneath.
2. Right-click the certificate → **Export** → format **Personal Information
   Exchange (.p12)**. Choose a strong export password and remember it.
3. Note the exact certificate name, including the team id in brackets:

```console
$ security find-identity -v -p codesigning
```

The quoted name on the `Developer ID Application` line is the signing identity.

## 3. Create a notarization API key

1. At <https://appstoreconnect.apple.com/access/integrations/api>, open
   **Team Keys** and generate a key with the **Developer** role.
2. Download the `AuthKey_XXXXXXXXXX.p8` file. Apple lets you download it once.
3. Note the **Key ID** (the `XXXXXXXXXX` part) and the **Issuer ID** shown at
   the top of that page.

## 4. Add the six secrets

Repository **Settings → Secrets and variables → Actions → New repository
secret**, or with the GitHub CLI from the repository folder. Run each line
separately (PowerShell-safe; the base64 lines are for macOS):

```console
$ base64 -i DeveloperID.p12 | gh secret set APPLE_DEVELOPER_ID_P12_BASE64
$ gh secret set APPLE_DEVELOPER_ID_P12_PASSWORD
$ gh secret set APPLE_SIGNING_IDENTITY
$ base64 -i AuthKey_XXXXXXXXXX.p8 | gh secret set APPLE_NOTARY_KEY_P8_BASE64
$ gh secret set APPLE_NOTARY_KEY_ID
$ gh secret set APPLE_NOTARY_ISSUER_ID
```

`gh secret set NAME` without a pipe prompts for the value, so it never lands in
your shell history.

| Secret | Value |
| --- | --- |
| `APPLE_DEVELOPER_ID_P12_BASE64` | The `.p12` file, base64-encoded |
| `APPLE_DEVELOPER_ID_P12_PASSWORD` | The export password from step 2 |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_NOTARY_KEY_P8_BASE64` | The `.p8` file, base64-encoded |
| `APPLE_NOTARY_KEY_ID` | The Key ID from step 3 |
| `APPLE_NOTARY_ISSUER_ID` | The Issuer ID from step 3 |

Setting the certificate secret but leaving another one out fails the run with
the name of the missing secret. It does not quietly ship an unsigned binary.

Afterwards, delete the `.p12` and `.p8` files from your Downloads folder, or
store them somewhere safe. Never commit them.

## 5. Dry run before tagging

On GitHub: **Actions → Release → Run workflow** (branch `main`). This builds
all three targets, signs and notarizes the macOS one, and uploads the archives
as run artifacts. It publishes nothing.

In the macOS job, the **Sign and notarize (macOS)** step should end with a
notarization result whose status is `Accepted`.

Download the macOS artifact and check it:

```console
$ codesign --verify --strict --verbose=2 ./graffy
$ codesign -dv --verbose=2 ./graffy
$ ./graffy --version
```

The second command should list your Developer ID under `Authority=` and show
`flags=0x10000(runtime)`.

## 6. Release as usual

Tag a release as before. The tag run signs, notarizes and publishes.

## Good to know

- **No stapling.** Apple's notarization ticket cannot be stapled to a bare
  executable, only to apps, disk images and installer packages. Gatekeeper
  therefore checks the binary online the first time it runs. A machine with no
  network on first run may still show a warning.
- **Expiry.** A Developer ID Application certificate lasts five years.
  Binaries signed while it was valid keep working; new releases need a new
  certificate and updated secrets.
- **If notarization is rejected**, the step prints Apple's result. The
  submission id in it can be passed to `xcrun notarytool log <id>` (with the
  same key options) to see why.
- **Linux binaries** are unaffected by any of this.
