#!/bin/sh
set -e

# Configuración del proveedor Proof-of-Origin (PO Token)
ENABLE_INTERNAL_POT="${ENABLE_INTERNAL_POT:-true}"
BGUTIL_POT_URL="${BGUTIL_POT_URL:-http://127.0.0.1:4416}"
export BGUTIL_POT_URL

if [ "$ENABLE_INTERNAL_POT" = "true" ] || [ "$ENABLE_INTERNAL_POT" = "1" ]; then
    case "$BGUTIL_POT_URL" in
        *127.0.0.1*|*localhost*)
            if [ -f "/opt/bgutil-pot/build/main.js" ]; then
                echo "[axAudio-entrypoint] Iniciando servidor interno bgutil-pot en 127.0.0.1:4416..."
                node /opt/bgutil-pot/build/main.js --host 127.0.0.1 --port 4416 &
                
                # Esperar hasta 5 segundos a que responda el endpoint /ping
                for i in $(seq 1 10); do
                    if curl -s http://127.0.0.1:4416/ping >/dev/null 2>&1; then
                        echo "[axAudio-entrypoint] bgutil-pot interno listo en 127.0.0.1:4416."
                        break
                    fi
                    sleep 0.5
                done
            else
                echo "[axAudio-entrypoint] Advertencia: /opt/bgutil-pot no encontrado, omitiendo servidor interno."
            fi
            ;;
        *)
            echo "[axAudio-entrypoint] BGUTIL_POT_URL configurado a servicio externo ($BGUTIL_POT_URL); servidor interno omitido."
            ;;
    esac
fi

# Iniciar proceso principal (axAudio)
exec "$@"
