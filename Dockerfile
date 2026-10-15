# ==========================================
# Stage 1: Build stage
# ==========================================
FROM rust:1-bookworm AS builder

# Instalar dependencias nativas de compilación (CMake, Opus, Pkg-config)
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    pkg-config \
    libopus-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 1. Copiar manifiestos para aprovechar el caché de capas de Docker
COPY Cargo.toml Cargo.lock ./

# 2. Crear un main.rs temporal para pre-compilar dependencias
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# 3. Copiar el código fuente real de axAudio
COPY src ./src

# 4. Compilar el binario final con Link-Time Optimization y versión inyectada
ARG AXAUDIO_VERSION="0.1.0"
ENV AXAUDIO_VERSION=$AXAUDIO_VERSION
RUN touch src/main.rs && cargo build --release

# ==========================================
# Stage 2: Minimal Runtime stage
# ==========================================
FROM debian:bookworm-slim AS runner

# Instalar dependencias mínimas de ejecución:
# - ca-certificates: esencial para streaming HTTPS de audios
# - libopus0: librería compartida de códec de audio
# - curl, unzip: utilidades
# - python3 & yt-dlp: extractor de audio y resolución de URLs CDN directas
# - deno: motor de JavaScript requerido por yt-dlp EJS para descifrar firmas de YouTube
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libopus0 \
    curl \
    unzip \
    python3 \
    python3-pip \
    && ARCH=$(uname -m) \
    && if [ "$ARCH" = "aarch64" ]; then DENO_ARCH="aarch64"; else DENO_ARCH="x86_64"; fi \
    && curl -fsSL "https://github.com/denoland/deno/releases/latest/download/deno-${DENO_ARCH}-unknown-linux-gnu.zip" -o /tmp/deno.zip \
    && unzip /tmp/deno.zip -d /usr/local/bin \
    && rm /tmp/deno.zip \
    && pip install --break-system-packages -U yt-dlp bgutil-ytdlp-pot-provider \
    && chmod a+rx /usr/local/bin/deno \
    && apt-get purge -y unzip \
    && rm -rf /var/lib/apt/lists/*

# Seguridad: crear usuario no root
RUN useradd -m -u 10001 -s /bin/sh axaudio
USER axaudio

WORKDIR /app

# Copiar el binario optimizado desde el builder
COPY --from=builder --chown=axaudio:axaudio /app/target/release/axAudio /app/axAudio

# Variables de entorno predeterminadas
ENV PORT=2333
ENV RUST_LOG=axAudio=info,tower_http=info

# Exponer puertos HTTP (2333 por defecto, 3000 para Coolify)
EXPOSE 2333 3000

# Monitoreo de salud dinámico que evalúa la variable $PORT
HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 \
    CMD sh -c 'curl -f "http://127.0.0.1:${PORT:-2333}/health" || exit 1'

# Ejecutar el motor de audio
CMD ["/app/axAudio"]
