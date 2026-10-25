# Names come from brand.json and nowhere else. Sourced by build-ipk.sh, the
# on-TV tool and the boot hook, so all three read the same file the same way.
#
#   brand_load <brand.json>   sets BRAND_DISPLAY_NAME, BRAND_SHORT_ID,
#                             BRAND_APP_ID, BRAND_DATA_DIR, BRAND_QML_MODULE,
#                             BRAND_REPO; returns 1 (message on stderr) if a
#                             value is missing or could not be used safely.
#
# The value checks are strict because the data dir and ids end up in rm -rf,
# sed expressions, systemd env files and QML import paths.

brand_get() {
  sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" "$2" 2>/dev/null | head -n1
}

brand_load() {
  BRAND_DISPLAY_NAME=$(brand_get displayName "$1")
  BRAND_SHORT_ID=$(brand_get shortId "$1")
  BRAND_APP_ID=$(brand_get appId "$1")
  BRAND_DATA_DIR=$(brand_get dataDir "$1")
  BRAND_QML_MODULE=$(brand_get qmlModule "$1")
  BRAND_REPO=$(brand_get repo "$1")
  case "$BRAND_DISPLAY_NAME" in ''|*[!A-Za-z0-9\ ]*) echo "brand.json: displayName must be letters, digits and spaces" >&2; return 1 ;; esac
  case "$BRAND_SHORT_ID" in ''|*[!a-z0-9]*) echo "brand.json: shortId must be lowercase letters and digits" >&2; return 1 ;; esac
  case "$BRAND_QML_MODULE" in ''|*[!a-z0-9]*) echo "brand.json: qmlModule must be lowercase letters and digits" >&2; return 1 ;; esac
  case "$BRAND_APP_ID" in ''|*[!a-z0-9.-]*) echo "brand.json: appId must be lowercase letters, digits, dots and dashes" >&2; return 1 ;; esac
  case "$BRAND_DATA_DIR" in
    /var/lib/?*) : ;;
    *) echo "brand.json: dataDir must be a directory directly under /var/lib" >&2; return 1 ;;
  esac
  case "${BRAND_DATA_DIR#/var/lib/}" in *[!a-z0-9-]*) echo "brand.json: dataDir must be /var/lib/<lowercase letters, digits, dashes>" >&2; return 1 ;; esac
  case "$BRAND_REPO" in *[!A-Za-z0-9._/-]*) echo "brand.json: repo has unusable characters" >&2; return 1 ;; esac
  return 0
}
