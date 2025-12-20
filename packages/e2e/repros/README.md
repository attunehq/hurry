# Reproduction Cases

This directory contains reproduction cases for bugs discovered during development. Each subdirectory is named after the PR or issue number where the bug was identified.

## Directory Structure

```
repros/
├── 295/           # PR #295: path-doubling bug in build script restoration
│   ├── ROOTCAUSE.md    # Root cause analysis
│   ├── repro.sh        # Script to reproduce the issue
│   ├── validate.sh     # Script to validate the fix
│   ├── Dockerfile      # Docker setup for reproduction
│   └── logs.tar.gz     # Compressed logs from investigation
└── README.md
```

## Naming Convention

Each repro directory is named by its associated PR or issue number:
- `295/` - PR #295
- `123/` - Issue #123

## Git LFS

Large files (like `*.tar.gz` log archives) are stored using Git LFS to keep the repository lightweight. Make sure you have Git LFS installed:

```bash
git lfs install
```

When cloning, LFS files are fetched automatically. To skip LFS files during clone:

```bash
GIT_LFS_SKIP_SMUDGE=1 git clone <repo-url>
```

## Sparse Checkout

If you want to clone the repository without the e2e package (to reduce checkout size), you can use sparse checkout:

```bash
# Clone with sparse checkout
git clone --filter=blob:none --sparse <repo-url>
cd <repo folder>

# Exclude the e2e package
git sparse-checkout set '/*' '!/packages/e2e'
```

To include the e2e package later:

```bash
git sparse-checkout add 'packages/e2e'
```
