# Makefile — local build and PyPI testing workflow
#
# Prerequisites:
#   pip install hatchling twine build
#   cargo build --profile release-small
#
# Typical workflow:
#   make dist          # build wheel + sdist for current platform
#   make check         # validate the distribution files
#   make install-local # install into current venv and smoke-test
#   make testpypi      # upload to TestPyPI
#   make test-testpypi # install from TestPyPI and verify
#   make publish       # upload to real PyPI (after confirming on TestPyPI)

PYTHON   := python3
PIP      := pip3
DIST_DIR := dist
PKG_DIR  := python
BINARY_NAME := bat_img_rs
BIN_NAME := bat_img

.PHONY: dist check install-local testpypi test-testpypi publish clean help

# ── Build ─────────────────────────────────────────────────────────────────────

## Build a wheel (current platform) + sdist
dist:
	@echo "==> Building Rust binary (release-small) …"
	cargo build --profile release-small
	@echo ""
	@echo "==> Rename binary (Unix)"
	cp target/release-small/$(BINARY_NAME) target/release-small/$(BIN_NAME)
	@echo ""
	@echo "==> Assembling wheel + sdist …"
	cd $(PKG_DIR) && $(PYTHON) build_wheels.py --local
	@echo ""
	@echo "Contents of dist/:"
	@ls -lh $(PKG_DIR)/$(DIST_DIR)/

# ── Validate ──────────────────────────────────────────────────────────────────

## Run twine check on all dist files (validates metadata + README rendering)
check: dist
	@echo "==> Running twine check …"
	$(PYTHON) -m twine check $(PKG_DIR)/$(DIST_DIR)/*
	@echo ""
	@echo "==> Checking wheel contents …"
	$(PYTHON) -m zipfile -l $(PKG_DIR)/$(DIST_DIR)/*.whl

# ── Local install ─────────────────────────────────────────────────────────────

## Install the local wheel into the active venv and run a smoke test
install-local: dist
	@echo "==> Installing local wheel …"
	$(PIP) install --force-reinstall $(PKG_DIR)/$(DIST_DIR)/*.whl
	@echo ""
	@echo "==> Smoke test (imgbatch --version) …"
	imgbatch --version
	@echo ""
	@echo "==> Smoke test (imgbatch --help) …"
	imgbatch --help

# ── TestPyPI ──────────────────────────────────────────────────────────────────

## Upload dist files to TestPyPI (https://test.pypi.org)
## Set TWINE_USERNAME and TWINE_PASSWORD, or use keyring / .pypirc
testpypi: check
	@echo "==> Uploading to TestPyPI …"
	$(PYTHON) -m twine upload \
		--repository testpypi \
		$(PKG_DIR)/$(DIST_DIR)/*
	@echo ""
	@echo "Package URL: https://test.pypi.org/project/imgbatch/"

## Install from TestPyPI and verify the CLI works
test-testpypi:
	@echo "==> Installing from TestPyPI …"
	$(PIP) install \
		--index-url https://test.pypi.org/simple/ \
		--extra-index-url https://pypi.org/simple/ \
		imgbatch
	@echo ""
	@echo "==> Verifying …"
	imgbatch --version
	imgbatch --help

# ── Real PyPI ─────────────────────────────────────────────────────────────────

## Upload to real PyPI — confirm you tested on TestPyPI first!
publish: check
	@echo "==> Uploading to PyPI …"
	$(PYTHON) -m twine upload $(PKG_DIR)/$(DIST_DIR)/*
	@echo ""
	@echo "Package URL: https://pypi.org/project/imgbatch/"

# ── Housekeeping ──────────────────────────────────────────────────────────────

## Remove build artifacts
clean:
	rm -rf $(PKG_DIR)/$(DIST_DIR)
	rm -rf $(PKG_DIR)/imgbatch_cli/__pycache__
	rm -rf $(PKG_DIR)/*.egg-info

## Show this help
help:
	@echo "Available targets:"
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## /  /'
	@echo ""
	@echo "Typical flow:"
	@echo "  make dist check install-local   # build + validate locally"
	@echo "  make testpypi test-testpypi     # test on TestPyPI"
	@echo "  make publish                    # release to real PyPI"
