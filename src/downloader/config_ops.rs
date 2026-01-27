//! Runtime configuration updates — speed limits, categories, schedule rules.

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Get the current global speed limit
    ///
    /// Returns the current speed limit in bytes per second, or None if unlimited.
    ///
    /// # Returns
    ///
    /// * `Option<u64>` - Speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// # let downloader = UsenetDownloader::new(config).await?;
    /// // Get current speed limit
    /// let limit = downloader.get_speed_limit();
    /// if let Some(bps) = limit {
    ///     println!("Current speed limit: {} bytes/sec", bps);
    /// } else {
    ///     println!("No speed limit (unlimited)");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_speed_limit(&self) -> Option<u64> {
        self.speed_limiter.get_limit()
    }

    /// Set the global speed limit
    ///
    /// This changes the download speed limit for all concurrent downloads.
    /// The change takes effect immediately.
    ///
    /// # Arguments
    ///
    /// * `limit_bps` - New speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// # let downloader = UsenetDownloader::new(config).await?;
    /// // Set to 10 MB/s
    /// downloader.set_speed_limit(Some(10_000_000)).await;
    ///
    /// // Remove speed limit (unlimited)
    /// downloader.set_speed_limit(None).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_speed_limit(&self, limit_bps: Option<u64>) {
        // Update the speed limiter
        self.speed_limiter.set_limit(limit_bps);

        // Emit event to notify subscribers
        self.emit_event(crate::types::Event::SpeedLimitChanged { limit_bps });

        tracing::info!(
            limit_bps = ?limit_bps,
            "Speed limit changed"
        );
    }

    /// Update runtime-changeable configuration settings
    ///
    /// This method updates configuration settings that can be safely changed while the
    /// downloader is running. Fields requiring restart (like database_path, download_dir,
    /// servers) cannot be updated via this method.
    ///
    /// # Arguments
    ///
    /// * `updates` - Configuration updates to apply
    pub async fn update_config(&self, updates: crate::config::ConfigUpdate) {
        // Update speed limit if provided
        if let Some(speed_limit) = updates.speed_limit_bps {
            self.set_speed_limit(speed_limit).await;
        }
    }

    /// Create or update a category
    ///
    /// This method adds a new category or updates an existing one with the provided configuration.
    /// The change takes effect immediately for new downloads.
    ///
    /// # Arguments
    ///
    /// * `name` - The category name
    /// * `config` - The category configuration
    pub async fn add_or_update_category(&self, name: &str, config: crate::config::CategoryConfig) {
        let mut categories = self.categories.write().await;
        categories.insert(name.to_string(), config);
    }

    /// Remove a category
    ///
    /// This method removes a category from the runtime configuration.
    /// Returns true if the category existed and was removed, false otherwise.
    ///
    /// # Arguments
    ///
    /// * `name` - The category name to remove
    ///
    /// # Returns
    ///
    /// `true` if the category was removed, `false` if it didn't exist
    pub async fn remove_category(&self, name: &str) -> bool {
        let mut categories = self.categories.write().await;
        categories.remove(name).is_some()
    }

    /// Get all categories
    ///
    /// Returns a clone of the current categories HashMap.
    pub async fn get_categories(
        &self,
    ) -> std::collections::HashMap<String, crate::config::CategoryConfig> {
        self.categories.read().await.clone()
    }

    // =========================================================================
    // Schedule Rule Management
    // =========================================================================

    /// Get all schedule rules
    ///
    /// Returns a clone of the current schedule rules list.
    pub async fn get_schedule_rules(&self) -> Vec<crate::config::ScheduleRule> {
        self.schedule_rules.read().await.clone()
    }

    /// Add a new schedule rule
    ///
    /// This method adds a new schedule rule to the runtime configuration.
    /// Returns the assigned rule ID.
    pub async fn add_schedule_rule(&self, rule: crate::config::ScheduleRule) -> i64 {
        let mut rules = self.schedule_rules.write().await;
        let id = self
            .next_schedule_rule_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        rules.push(rule);
        id
    }

    /// Update an existing schedule rule
    ///
    /// This method updates a schedule rule at the specified index.
    /// Returns true if the rule was updated, false if the index was invalid.
    pub async fn update_schedule_rule(&self, id: i64, rule: crate::config::ScheduleRule) -> bool {
        let mut rules = self.schedule_rules.write().await;
        if let Some(r) = rules.get_mut(id as usize) {
            *r = rule;
            true
        } else {
            false
        }
    }

    /// Remove a schedule rule
    ///
    /// This method removes a schedule rule at the specified index.
    /// Returns true if the rule was removed, false if the index was invalid.
    pub async fn remove_schedule_rule(&self, id: i64) -> bool {
        let mut rules = self.schedule_rules.write().await;
        if (id as usize) < rules.len() {
            rules.remove(id as usize);
            true
        } else {
            false
        }
    }
}
