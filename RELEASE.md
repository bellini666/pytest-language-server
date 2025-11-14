# Release Process

## PyPI Trusted Publishing Setup

To enable automatic publishing to PyPI, you need to configure trusted publishing on PyPI:

1. Go to https://pypi.org/manage/project/pytest-language-server/settings/publishing/
2. Click "Add a new publisher"
3. Configure the following:
   - **PyPI Project Name**: `pytest-language-server`
   - **Owner**: `bellini666`
   - **Repository name**: `pytest-language-server`
   - **Workflow name**: `release.yml`
   - **Environment name**: (leave empty)

This allows the GitHub Actions workflow to publish to PyPI without requiring an API token.

## Creating a Release

1. Ensure all changes are committed and pushed to `master`
2. Run tests locally: `cargo test`
3. Create and push a version tag:
   ```bash
   git tag v0.X.Y
   git push origin v0.X.Y
   ```
4. The GitHub Actions workflow will automatically:
   - Build wheels for all platforms (Linux, macOS, Windows)
   - Build for multiple Python versions (3.10-3.14, 3.14t, PyPy)
   - Generate artifact attestations for supply chain security
   - Create a GitHub release with all wheels
   - Publish to PyPI
   - Publish to crates.io (requires `CARGO_REGISTRY_TOKEN` secret)

## Crates.io Publishing

The workflow also publishes to crates.io. Ensure the `CARGO_REGISTRY_TOKEN` secret is set:

1. Generate a token at https://crates.io/me/tokens
2. Add it to GitHub repository secrets as `CARGO_REGISTRY_TOKEN`

## Homebrew Formula

After a release, update the Homebrew formula SHA256 hashes:

1. Download the wheels from the GitHub release
2. Calculate SHA256 for each platform:
   ```bash
   shasum -a 256 pytest_language_server-X.Y.Z-*.whl
   ```
3. Update `Formula/pytest-language-server.rb` with the actual hashes
4. Commit and push the updated formula

## Troubleshooting

### PyPI Upload Fails

- Verify trusted publishing is configured correctly on PyPI
- Check that the workflow has `id-token: write` permission
- Ensure package version doesn't already exist on PyPI

### GitHub Release Upload Issues

- The workflow uses `softprops/action-gh-release` which may show "Not Found" errors when trying to delete non-existent assets
- These errors are usually harmless if the upload succeeds afterward
- The workflow now flattens the directory structure to avoid glob pattern issues

### Wheels Missing

- Check that all build jobs completed successfully
- Verify artifact upload/download worked correctly
- Look for build errors in the CI logs for specific platforms
