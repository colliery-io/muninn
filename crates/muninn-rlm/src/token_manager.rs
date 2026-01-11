//! Token management for OAuth tokens.
//!
//! Handles saving, loading, and refreshing OAuth tokens for the
//! Anthropic MAX plan authentication.
//!
//! This module provides:
//! - `TokenManager` trait for abstracting token storage/retrieval
//! - `FileTokenManager` for file-based persistence (production use)
//! - `InMemoryTokenManager` for testing without filesystem dependencies

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::error::{Result, RlmError};
use crate::oauth::{OAuthConfig, OAuthTokens, refresh_access_token};

/// Default token file name within the .muninn directory.
pub const TOKEN_FILE: &str = "oauth-tokens.json";

/// Buffer time before expiry to trigger refresh (5 minutes in milliseconds).
const REFRESH_BUFFER_MS: u64 = 5 * 60 * 1000;

// ============================================================================
// TokenManager Trait
// ============================================================================

/// Trait for managing OAuth token lifecycle.
///
/// This trait abstracts token storage and retrieval to enable testing
/// without filesystem dependencies.
#[async_trait]
pub trait TokenManager: Send + Sync + std::fmt::Debug {
    /// Get a valid access token, refreshing if necessary.
    async fn get_valid_access_token(&self) -> Result<String>;

    /// Check if tokens exist.
    fn has_tokens(&self) -> bool;

    /// Save tokens to storage.
    async fn save_tokens(&self, tokens: &OAuthTokens) -> Result<()>;

    /// Load tokens from storage.
    async fn load_tokens(&self) -> Result<Option<OAuthTokens>>;

    /// Delete stored tokens.
    async fn delete_tokens(&self) -> Result<()>;

    /// Clear cached tokens (useful for forcing reload).
    async fn clear_cache(&self);

    /// Get token expiry information for display.
    async fn get_token_info(&self) -> Result<Option<TokenInfo>>;
}

// ============================================================================
// FileTokenManager
// ============================================================================

/// File-based token manager for production use.
///
/// Persists tokens to a JSON file on disk.
#[derive(Debug)]
pub struct FileTokenManager {
    /// Path to the token file.
    token_path: PathBuf,
    /// OAuth configuration.
    config: OAuthConfig,
    /// Cached tokens (with RwLock for concurrent access).
    cached_tokens: Arc<RwLock<Option<OAuthTokens>>>,
}

impl FileTokenManager {
    /// Create a new file-based token manager.
    ///
    /// # Arguments
    /// * `muninn_dir` - Path to the .muninn directory
    pub fn new(muninn_dir: &Path) -> Self {
        Self {
            token_path: muninn_dir.join(TOKEN_FILE),
            config: OAuthConfig::default(),
            cached_tokens: Arc::new(RwLock::new(None)),
        }
    }

    /// Create a token manager with a custom token path.
    pub fn with_path(token_path: PathBuf) -> Self {
        Self {
            token_path,
            config: OAuthConfig::default(),
            cached_tokens: Arc::new(RwLock::new(None)),
        }
    }

    /// Get the token file path.
    pub fn token_path(&self) -> &Path {
        &self.token_path
    }

    /// Check if tokens are expired (with buffer time).
    pub fn is_token_expired(tokens: &OAuthTokens) -> bool {
        if tokens.expires_at == 0 {
            return true;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        now >= tokens.expires_at.saturating_sub(REFRESH_BUFFER_MS)
    }
}

#[async_trait]
impl TokenManager for FileTokenManager {
    fn has_tokens(&self) -> bool {
        self.token_path.exists()
    }

    async fn save_tokens(&self, tokens: &OAuthTokens) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.token_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RlmError::Config(format!("Failed to create token directory: {}", e))
            })?;
        }

        let json = serde_json::to_string_pretty(tokens)
            .map_err(|e| RlmError::Serialization(format!("Failed to serialize tokens: {}", e)))?;

        std::fs::write(&self.token_path, json)
            .map_err(|e| RlmError::Config(format!("Failed to write token file: {}", e)))?;

        // Update cache
        let mut cache = self.cached_tokens.write().await;
        *cache = Some(tokens.clone());

        tracing::info!("Tokens saved to {}", self.token_path.display());
        Ok(())
    }

    async fn load_tokens(&self) -> Result<Option<OAuthTokens>> {
        // Check cache first
        {
            let cache = self.cached_tokens.read().await;
            if cache.is_some() {
                return Ok(cache.clone());
            }
        }

        // Load from disk
        if !self.token_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&self.token_path)
            .map_err(|e| RlmError::Config(format!("Failed to read token file: {}", e)))?;

        let tokens: OAuthTokens = serde_json::from_str(&content)
            .map_err(|e| RlmError::Serialization(format!("Failed to parse token file: {}", e)))?;

        // Update cache
        let mut cache = self.cached_tokens.write().await;
        *cache = Some(tokens.clone());

        Ok(Some(tokens))
    }

    async fn get_valid_access_token(&self) -> Result<String> {
        let tokens = self.load_tokens().await?.ok_or_else(|| {
            RlmError::Config("No OAuth tokens found. Run 'muninn oauth' first.".to_string())
        })?;

        if Self::is_token_expired(&tokens) {
            tracing::info!("Token expired, refreshing...");
            let mut new_tokens = refresh_access_token(&self.config, &tokens.refresh_token).await?;

            // Preserve refresh token if not returned in response
            if new_tokens.refresh_token.is_empty() {
                new_tokens.refresh_token = tokens.refresh_token;
            }

            self.save_tokens(&new_tokens).await?;
            tracing::info!("Token refreshed successfully");
            return Ok(new_tokens.access_token);
        }

        Ok(tokens.access_token)
    }

    async fn clear_cache(&self) {
        let mut cache = self.cached_tokens.write().await;
        *cache = None;
    }

    async fn delete_tokens(&self) -> Result<()> {
        if self.token_path.exists() {
            std::fs::remove_file(&self.token_path)
                .map_err(|e| RlmError::Config(format!("Failed to delete token file: {}", e)))?;
        }
        self.clear_cache().await;
        Ok(())
    }

    async fn get_token_info(&self) -> Result<Option<TokenInfo>> {
        let tokens = self.load_tokens().await?;
        match tokens {
            Some(t) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;

                let expires_in_secs = if t.expires_at > now {
                    (t.expires_at - now) / 1000
                } else {
                    0
                };

                let is_expired = Self::is_token_expired(&t);
                Ok(Some(TokenInfo {
                    created_at: t.created_at,
                    expires_in_secs,
                    is_expired,
                    scope: t.scope,
                }))
            }
            None => Ok(None),
        }
    }
}

// ============================================================================
// InMemoryTokenManager
// ============================================================================

/// In-memory token manager for testing.
///
/// Does not persist tokens to disk; stores them in memory only.
/// Useful for unit tests that need to verify OAuth flows without
/// filesystem dependencies.
#[derive(Debug)]
pub struct InMemoryTokenManager {
    /// Cached tokens.
    tokens: RwLock<Option<OAuthTokens>>,
    /// Count of refresh operations (for testing assertions).
    refresh_count: AtomicU32,
}

impl InMemoryTokenManager {
    /// Create a new empty in-memory token manager.
    pub fn new() -> Self {
        Self {
            tokens: RwLock::new(None),
            refresh_count: AtomicU32::new(0),
        }
    }

    /// Create an in-memory token manager pre-loaded with tokens.
    pub fn with_tokens(tokens: OAuthTokens) -> Self {
        Self {
            tokens: RwLock::new(Some(tokens)),
            refresh_count: AtomicU32::new(0),
        }
    }

    /// Get the number of refresh operations performed.
    pub fn refresh_count(&self) -> u32 {
        self.refresh_count.load(Ordering::SeqCst)
    }

    /// Check if tokens are expired (with buffer time).
    pub fn is_token_expired(tokens: &OAuthTokens) -> bool {
        FileTokenManager::is_token_expired(tokens)
    }
}

impl Default for InMemoryTokenManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TokenManager for InMemoryTokenManager {
    fn has_tokens(&self) -> bool {
        // Use try_read to avoid blocking; if we can't get the lock, assume tokens exist
        self.tokens
            .try_read()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    async fn save_tokens(&self, tokens: &OAuthTokens) -> Result<()> {
        let mut cache = self.tokens.write().await;
        *cache = Some(tokens.clone());
        Ok(())
    }

    async fn load_tokens(&self) -> Result<Option<OAuthTokens>> {
        let cache = self.tokens.read().await;
        Ok(cache.clone())
    }

    async fn get_valid_access_token(&self) -> Result<String> {
        let tokens = self
            .load_tokens()
            .await?
            .ok_or_else(|| RlmError::Config("No OAuth tokens available".to_string()))?;

        if Self::is_token_expired(&tokens) {
            // In testing, we simulate refresh by just incrementing the counter
            // and returning the current token (real refresh needs network)
            self.refresh_count.fetch_add(1, Ordering::SeqCst);
            tracing::debug!("InMemoryTokenManager: simulated token refresh");
        }

        Ok(tokens.access_token)
    }

    async fn clear_cache(&self) {
        // For in-memory, clearing cache means clearing tokens
        let mut cache = self.tokens.write().await;
        *cache = None;
    }

    async fn delete_tokens(&self) -> Result<()> {
        self.clear_cache().await;
        Ok(())
    }

    async fn get_token_info(&self) -> Result<Option<TokenInfo>> {
        let tokens = self.load_tokens().await?;
        match tokens {
            Some(t) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;

                let expires_in_secs = if t.expires_at > now {
                    (t.expires_at - now) / 1000
                } else {
                    0
                };

                let is_expired = Self::is_token_expired(&t);
                Ok(Some(TokenInfo {
                    created_at: t.created_at,
                    expires_in_secs,
                    is_expired,
                    scope: t.scope,
                }))
            }
            None => Ok(None),
        }
    }
}

// ============================================================================
// TokenInfo
// ============================================================================

/// Information about stored tokens for display.
#[derive(Debug, Clone)]
pub struct TokenInfo {
    /// When the tokens were created.
    pub created_at: String,
    /// Seconds until expiry.
    pub expires_in_secs: u64,
    /// Whether the token is expired or will expire soon.
    pub is_expired: bool,
    /// Granted scopes.
    pub scope: String,
}

impl TokenInfo {
    /// Format expiry time for display.
    pub fn expires_in_display(&self) -> String {
        if self.is_expired {
            "Expired (will refresh on next use)".to_string()
        } else {
            let hours = self.expires_in_secs / 3600;
            let minutes = (self.expires_in_secs % 3600) / 60;
            format!("{}h {}m", hours, minutes)
        }
    }
}

// ============================================================================
// Shared Token Manager
// ============================================================================

/// Shared token manager for use across async contexts.
///
/// This is a trait object allowing either FileTokenManager or InMemoryTokenManager
/// to be used interchangeably.
pub type SharedTokenManager = Arc<dyn TokenManager>;

/// Create a shared file-based token manager.
pub fn create_token_manager(muninn_dir: &Path) -> SharedTokenManager {
    Arc::new(FileTokenManager::new(muninn_dir))
}

/// Create a shared in-memory token manager (for testing).
pub fn create_memory_token_manager() -> SharedTokenManager {
    Arc::new(InMemoryTokenManager::new())
}

/// Create a shared in-memory token manager with pre-loaded tokens (for testing).
pub fn create_memory_token_manager_with_tokens(tokens: OAuthTokens) -> SharedTokenManager {
    Arc::new(InMemoryTokenManager::with_tokens(tokens))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ========================================================================
    // FileTokenManager Tests
    // ========================================================================

    #[tokio::test]
    async fn test_file_token_manager_new() {
        let temp = tempdir().unwrap();
        let manager = FileTokenManager::new(temp.path());
        assert!(!manager.has_tokens());
    }

    #[tokio::test]
    async fn test_file_save_and_load_tokens() {
        let temp = tempdir().unwrap();
        let manager = FileTokenManager::new(temp.path());

        let tokens = OAuthTokens {
            access_token: "test_access".to_string(),
            refresh_token: "test_refresh".to_string(),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: 9999999999999,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        manager.save_tokens(&tokens).await.unwrap();
        assert!(manager.has_tokens());

        let loaded = manager.load_tokens().await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "test_access");
        assert_eq!(loaded.refresh_token, "test_refresh");
    }

    #[test]
    fn test_is_token_expired() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Token that expires in 1 hour - not expired
        let valid_tokens = OAuthTokens {
            access_token: "test".to_string(),
            refresh_token: "test".to_string(),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: now + 3600 * 1000,
            created_at: "".to_string(),
        };
        assert!(!FileTokenManager::is_token_expired(&valid_tokens));

        // Token that expires in 2 minutes - should be considered expired (within buffer)
        let expiring_tokens = OAuthTokens {
            access_token: "test".to_string(),
            refresh_token: "test".to_string(),
            expires_in: 120,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: now + 2 * 60 * 1000,
            created_at: "".to_string(),
        };
        assert!(FileTokenManager::is_token_expired(&expiring_tokens));

        // Token that already expired
        let expired_tokens = OAuthTokens {
            access_token: "test".to_string(),
            refresh_token: "test".to_string(),
            expires_in: 0,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: now - 1000,
            created_at: "".to_string(),
        };
        assert!(FileTokenManager::is_token_expired(&expired_tokens));
    }

    #[tokio::test]
    async fn test_file_delete_tokens() {
        let temp = tempdir().unwrap();
        let manager = FileTokenManager::new(temp.path());

        let tokens = OAuthTokens {
            access_token: "test".to_string(),
            refresh_token: "test".to_string(),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: 9999999999999,
            created_at: "".to_string(),
        };

        manager.save_tokens(&tokens).await.unwrap();
        assert!(manager.has_tokens());

        manager.delete_tokens().await.unwrap();
        assert!(!manager.has_tokens());
    }

    // ========================================================================
    // InMemoryTokenManager Tests
    // ========================================================================

    #[tokio::test]
    async fn test_inmemory_token_manager_new() {
        let manager = InMemoryTokenManager::new();
        assert!(!manager.has_tokens());
        assert_eq!(manager.refresh_count(), 0);
    }

    #[tokio::test]
    async fn test_inmemory_with_tokens() {
        let tokens = OAuthTokens {
            access_token: "test_access".to_string(),
            refresh_token: "test_refresh".to_string(),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: 9999999999999,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let manager = InMemoryTokenManager::with_tokens(tokens);
        assert!(manager.has_tokens());

        let loaded = manager.load_tokens().await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "test_access");
    }

    #[tokio::test]
    async fn test_inmemory_save_and_load() {
        let manager = InMemoryTokenManager::new();

        let tokens = OAuthTokens {
            access_token: "saved_token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: 9999999999999,
            created_at: "".to_string(),
        };

        manager.save_tokens(&tokens).await.unwrap();
        assert!(manager.has_tokens());

        let loaded = manager.load_tokens().await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "saved_token");
    }

    #[tokio::test]
    async fn test_inmemory_get_valid_access_token() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let tokens = OAuthTokens {
            access_token: "valid_token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: now + 3600 * 1000, // 1 hour from now
            created_at: "".to_string(),
        };

        let manager = InMemoryTokenManager::with_tokens(tokens);
        let token = manager.get_valid_access_token().await.unwrap();
        assert_eq!(token, "valid_token");
        assert_eq!(manager.refresh_count(), 0); // No refresh needed
    }

    #[tokio::test]
    async fn test_inmemory_refresh_count_on_expired() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let tokens = OAuthTokens {
            access_token: "expired_token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_in: 0,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: now - 1000, // Already expired
            created_at: "".to_string(),
        };

        let manager = InMemoryTokenManager::with_tokens(tokens);
        let _ = manager.get_valid_access_token().await.unwrap();
        assert_eq!(manager.refresh_count(), 1); // Refresh was simulated
    }

    #[tokio::test]
    async fn test_inmemory_delete_tokens() {
        let tokens = OAuthTokens {
            access_token: "test".to_string(),
            refresh_token: "test".to_string(),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: 9999999999999,
            created_at: "".to_string(),
        };

        let manager = InMemoryTokenManager::with_tokens(tokens);
        assert!(manager.has_tokens());

        manager.delete_tokens().await.unwrap();
        assert!(!manager.has_tokens());
    }

    #[tokio::test]
    async fn test_inmemory_no_tokens_error() {
        let manager = InMemoryTokenManager::new();
        let result = manager.get_valid_access_token().await;
        assert!(result.is_err());
    }

    // ========================================================================
    // Trait Object Tests
    // ========================================================================

    #[tokio::test]
    async fn test_shared_token_manager_file() {
        let temp = tempdir().unwrap();
        let manager: SharedTokenManager = create_token_manager(temp.path());
        assert!(!manager.has_tokens());
    }

    #[tokio::test]
    async fn test_shared_token_manager_memory() {
        let manager: SharedTokenManager = create_memory_token_manager();
        assert!(!manager.has_tokens());
    }

    #[tokio::test]
    async fn test_shared_token_manager_with_tokens() {
        let tokens = OAuthTokens {
            access_token: "shared_token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            scope: "test".to_string(),
            expires_at: 9999999999999,
            created_at: "".to_string(),
        };

        let manager: SharedTokenManager = create_memory_token_manager_with_tokens(tokens);
        assert!(manager.has_tokens());

        let loaded = manager.load_tokens().await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "shared_token");
    }
}
