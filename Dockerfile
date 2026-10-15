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

# 4. Compilar el binario final con Link-Time Optimization
RUN touch src/main.rs && cargo build --release

# ==========================================
# Stage 2: Minimal Runtime stage
# ==========================================
FROM debian:bookworm-slim AS runner

# Instalar dependencias mínimas de ejecución:
# - ca-certificates: esencial para streaming HTTPS de audios
# - libopus0: librería compartida de códec de audio
# - curl: para el healthcheck de Docker/Coolify
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libopus0 \
    curl \
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

# Exponer el puerto HTTP de control
EXPOSE 2333

# Monitoreo de salud para Docker / Coolify
HEALTHCHECK --interval=20s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://127.0.0.1:2333/health || exit 1

# Ejecutar el motor de audio
CMD ["/app/axAudio"]
