#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PatternMode {
    Manual,
    Spiral,
    CustomFile,
}

impl Default for PatternMode {
    fn default() -> Self {
        Self::Manual
    }
}

/// Application configuration and simulation parameters in normalized space.
/// Normalized space scales from 0.0 to 1.0 relative to the sand table radius.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// Speed of the marble in units of radius per second (0.01 to 2.0).
    pub speed: f32,
    /// Size (radius) of the marble as a fraction of the table radius (0.005 to 0.1).
    pub marble_size: f32,
    /// Spacing between spiral turns as a fraction of the table radius (0.005 to 0.2).
    pub spiral_spacing: f32,
    /// Flag to enable auto-play of the spiral pattern.
    pub auto_play: bool,
    /// Light brightness slider (0.0 to 1.0).
    pub light_brightness: f32,
    /// Selection of current drawing pattern source.
    pub pattern_mode: PatternMode,
    /// Path to a custom .thr or .gcode pattern file.
    pub custom_file_path: String,
    /// Light angle around the circular bed in radians (0.0 to 2 * PI).
    pub light_angle: f32,
    /// Light RGB color components.
    pub light_color: [f32; 3],
    /// Sand RGB base color components.
    pub sand_color: [f32; 3],
    /// Flag to enable GPU raymarched shadows.
    pub shadows_enabled: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            speed: 0.15,
            marble_size: 0.025,
            spiral_spacing: 0.030,
            auto_play: false,
            light_brightness: 0.8,
            pattern_mode: PatternMode::Manual,
            custom_file_path: String::new(),
            light_angle: 0.7853982, // ~45 degrees in radians
            light_color: [1.0, 0.95, 0.82], // Warm golden white
            sand_color: [0.92, 0.89, 0.82], // Warm sandy beige
            shadows_enabled: true,
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.speed, 0.15);
        assert_eq!(config.marble_size, 0.025);
        assert_eq!(config.spiral_spacing, 0.030);
        assert_eq!(config.light_brightness, 0.8);
        assert!(!config.auto_play);
        assert_eq!(config.pattern_mode, PatternMode::Manual);
        assert_eq!(config.custom_file_path, "");
        assert_eq!(config.light_angle, 0.7853982);
        assert_eq!(config.light_color, [1.0, 0.95, 0.82]);
        assert_eq!(config.sand_color, [0.92, 0.89, 0.82]);
        assert!(config.shadows_enabled);
    }

    #[test]
    fn test_serialization() {
        let config = AppConfig::default();
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: AppConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_json_schema_stability() {
        let json_str = r#"{"speed":0.15,"marble_size":0.025,"spiral_spacing":0.03,"auto_play":false,"light_brightness":0.8,"pattern_mode":"Manual","custom_file_path":"","light_angle":0.7853982,"light_color":[1.0,0.95,0.82],"sand_color":[0.92,0.89,0.82],"shadows_enabled":true}"#;
        let deserialized: AppConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(deserialized, AppConfig::default());
    }
}
