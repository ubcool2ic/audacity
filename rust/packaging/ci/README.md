# CI workflow

`rust-packaging.yml` is the GitHub Actions workflow for audacity-rs (build, Xvfb tests, .deb/AppImage, .msi, .dmg, smoke tests).
It must live at `.github/workflows/rust-packaging.yml`; the agent token cannot push workflow files (no `workflow` scope), so a maintainer has to copy it there:

    cp rust/packaging/ci/rust-packaging.yml .github/workflows/

Signing secrets (optional): WINDOWS_CERT_PFX_BASE64, WINDOWS_CERT_PASSWORD, MACOS_CERT_P12_BASE64, MACOS_CERT_PASSWORD, MACOS_SIGN_IDENTITY, APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD.
