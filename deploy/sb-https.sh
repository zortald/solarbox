# Sourced by push-ui.sh / push-config.sh: HTTPS to solarbox with its
# self-signed certificate pinned by public key (SB_PIN in deploy/local.env).
# Sets SB_HTTPS (base URL) and SB_TLS (curl options).
SB_HTTPS=$(printf %s "${SB_URL:?set SB_URL (deploy/local.env)}" | sed 's#^http://#https://#')
if [ -z "$SB_PIN" ]; then
  hp=$(printf %s "$SB_HTTPS" | sed 's#^https://##; s#/.*##')
  case $hp in *:*) ;; *) hp=$hp:443 ;; esac
  pin=$(openssl s_client -connect "$hp" </dev/null 2>/dev/null | openssl x509 -pubkey -noout | openssl pkey -pubin -outform der | openssl dgst -sha256 -binary | base64)
  echo "SB_PIN is not set. solarbox at $hp presents key pin:" >&2
  echo "  SB_PIN=$pin" >&2
  echo "Compare the certificate fingerprint with the «Доступ» page, then add that line to deploy/local.env." >&2
  exit 1
fi
SB_TLS="-k --pinnedpubkey sha256//$SB_PIN"
