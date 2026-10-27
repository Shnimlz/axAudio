//! Rate limiter para proteger axAudio de abuso y sobrecarga
//!
//! - Limita requests por guild_id (previene un servidor de saturar el servicio)
//! - Limita subprocesses concurrentes de yt-dlp (previene consumo de CPU)
//! - Configurable via environment variables

use std::collections::HashMap;
use std::time::Instant;
use std::sync::{Arc, RwLock};
use tracing::warn;

/// Configuración de rate limiting (se puede cambiar via env vars)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RateLimitConfig {
    /// Requests máximas por guild para operaciones costosas (resolve, search)
    pub max_requests_per_minute: u32,
    
    /// Joins máximos por guild por minuto
    pub max_joins_per_minute: u32,
    
    /// Subprocesses de yt-dlp concurrentes máximos
    pub max_concurrent_ytdlp: u32,
    
    /// Requests máximas por guild para operaciones baratas (stats, queue, etc.)
    pub max_light_requests_per_minute: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests_per_minute: 30,        // 30 resolves/search por guild/min
            max_joins_per_minute: 10,           // 10 voice joins por guild/min
            max_concurrent_ytdlp: 5,            // Máximo 5 yt-dlp a la vez
            max_light_requests_per_minute: 60,  // 60 operaciones ligeras por guild/min
        }
    }
}

impl RateLimitConfig {
    /// Carga configuración desde environment variables
    pub fn from_env() -> Self {
        Self {
            max_requests_per_minute: std::env::var("RATE_LIMIT_RESOLVE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            
            max_joins_per_minute: std::env::var("RATE_LIMIT_JOINS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            
            max_concurrent_ytdlp: std::env::var("RATE_LIMIT_YTDLP")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            
            max_light_requests_per_minute: std::env::var("RATE_LIMIT_LIGHT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
        }
    }
}

/// Contador de requests por guild (limpia automáticamente entries viejos)
#[derive(Debug, Clone)]
pub struct GuildCounter {
    requests: Vec<Instant>,
}

impl GuildCounter {
    pub fn new() -> Self {
        Self {
            requests: Vec::with_capacity(100),
        }
    }

    /// Registra una request y retorna cuántas hay en el último minuto
    pub fn record(&mut self) -> u32 {
        let now = Instant::now();
        self.requests.push(now);
        
        // Limpia requests de hace más de 1 minuto
        let cutoff = now
            .checked_sub(std::time::Duration::from_secs(60))
            .unwrap_or_else(|| now - std::time::Duration::from_secs(3600));
        
        self.requests.retain(|t| *t > cutoff);
        self.requests.len() as u32
    }

    /// Registra un join y retorna cuántos hay en el último minuto
    pub fn record_join(&mut self) -> u32 {
        let now = Instant::now();
        self.requests.push(now);
        
        let cutoff = now
            .checked_sub(std::time::Duration::from_secs(60))
            .unwrap_or_else(|| now - std::time::Duration::from_secs(3600));
        
        self.requests.retain(|t| *t > cutoff);
        self.requests.len() as u32
    }

    /// Limpia entries viejos (para el cleanup periódico)
    #[allow(dead_code)]
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        let cutoff = now
            .checked_sub(std::time::Duration::from_secs(60))
            .unwrap_or_else(|| now - std::time::Duration::from_secs(3600));
        
        self.requests.retain(|t| *t > cutoff);
    }
}

/// Rate limiter principal
#[derive(Clone)]
pub struct RateLimiter {
    config: RateLimitConfig,
    counters: Arc<RwLock<HashMap<u64, GuildCounter>>>,
    #[allow(dead_code)]
    concurrent_ytdlp: Arc<std::sync::Mutex<u32>>,
}

#[allow(dead_code)]
impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config: config.clone(),
            counters: Arc::new(RwLock::new(HashMap::new())),
            concurrent_ytdlp: Arc::new(std::sync::Mutex::new(0)),
        }
    }

    /// Checkea si un guild puede hacer una operación de tipo "resolve/search"
    pub fn check_resolve(&self, guild_id: u64) -> Result<(), RateLimitError> {
        let mut counters = self.counters.write().unwrap();
        let counter = counters.entry(guild_id).or_insert_with(GuildCounter::new);
        
        let count = counter.record();
        if count > self.config.max_requests_per_minute {
            warn!(
                guild = guild_id,
                count = count,
                limit = self.config.max_requests_per_minute,
                "Rate limit excedido para resolve/search"
            );
            return Err(RateLimitError::TooManyRequests);
        }
        
        Ok(())
    }

    /// Checkea si un guild puede hacer un voice join
    pub fn check_join(&self, guild_id: u64) -> Result<(), RateLimitError> {
        let mut counters = self.counters.write().unwrap();
        let counter = counters.entry(guild_id).or_insert_with(GuildCounter::new);
        
        let count = counter.record_join();
        if count > self.config.max_joins_per_minute {
            warn!(
                guild = guild_id,
                count = count,
                limit = self.config.max_joins_per_minute,
                "Rate limit excedido para voice joins"
            );
            return Err(RateLimitError::TooManyJoins);
        }
        
        Ok(())
    }

    /// Checkea si un guild puede hacer una operación ligera (stats, queue, etc.)
    pub fn check_light(&self, guild_id: u64) -> Result<(), RateLimitError> {
        let mut counters = self.counters.write().unwrap();
        let counter = counters.entry(guild_id).or_insert_with(GuildCounter::new);
        
        let count = counter.record();
        if count > self.config.max_light_requests_per_minute {
            warn!(
                guild = guild_id,
                count = count,
                limit = self.config.max_light_requests_per_minute,
                "Rate limit excedido para operaciones ligeras"
            );
            return Err(RateLimitError::TooManyRequests);
        }
        
        Ok(())
    }

    /// Intenta adquirir un slot de yt-dlp concurrente
    /// Retorna Ok(()) si se pudo adquirir, Err(()) si se excedió el límite
    pub fn try_acquire_ytdlp(&self) -> Result<(), ()> {
        let mut concurrent = self.concurrent_ytdlp.lock().unwrap();
        
        if *concurrent >= self.config.max_concurrent_ytdlp {
            warn!(
                current = *concurrent,
                max = self.config.max_concurrent_ytdlp,
                "yt-dlp concurrent limit reached"
            );
            Err(())
        } else {
            *concurrent += 1;
            Ok(())
        }
    }

    /// Libera un slot de yt-dlp (llamar cuando el subprocess termina)
    pub fn release_ytdlp(&self) {
        let mut concurrent = self.concurrent_ytdlp.lock().unwrap();
        if *concurrent > 0 {
            *concurrent -= 1;
        }
    }

    /// Limpia counters de guilds inactivos
    pub fn cleanup_counters(&self) {
        let mut counters = self.counters.write().unwrap();
        
        counters.retain(|_guild_id, counter| {
            let before = counter.requests.len();
            counter.cleanup();
            let after = counter.requests.len();
            
            if before > 0 && after == 0 {
                false // Eliminar counter vacío
            } else {
                true
            }
        });
    }

    /// Retorna la configuración actual (para monitoreo)
    pub fn config(&self) -> &RateLimitConfig {
        &self.config
    }
}

/// Error de rate limiting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitError {
    /// Demasiadas requests para resolve/search
    TooManyRequests,
    
    /// Demasiados voice joins
    TooManyJoins,
}

impl std::fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RateLimitError::TooManyRequests => write!(f, "Demasiadas requests"),
            RateLimitError::TooManyJoins => write!(f, "Demasiados voice joins"),
        }
    }
}

impl std::error::Error for RateLimitError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_per_minute() {
        let limiter = RateLimiter::new(RateLimitConfig::default());
        let guild_id = 123456;
        
        // Debería permitir hasta el límite
        for _ in 0..30 {
            assert!(limiter.check_resolve(guild_id).is_ok());
        }
        
        // El siguiente debería fallar
        assert!(limiter.check_resolve(guild_id).is_err());
    }

    #[test]
    fn test_join_rate_limit() {
        let config = RateLimitConfig {
            max_joins_per_minute: 5,
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);
        let guild_id = 789;
        
        for _ in 0..5 {
            assert!(limiter.check_join(guild_id).is_ok());
        }
        
        assert!(limiter.check_join(guild_id).is_err());
    }

    #[test]
    fn test_different_guilds_independent() {
        let limiter = RateLimiter::new(RateLimitConfig::default());
        
        // Llenar guild 1
        for _ in 0..30 {
            let _ = limiter.check_resolve(111);
        }
        
        // Guild 2 debería seguir funcionando
        assert!(limiter.check_resolve(222).is_ok());
    }
}
