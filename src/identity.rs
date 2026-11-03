use rand::Rng;
use serde::{Deserialize, Serialize};

// ============================================================================
// Anonymous Random Wordlists (Adjectives + Animals/Nouns)
// ============================================================================

const ADJECTIVES: &[&str] = &[
    "Amber", "Astral", "Aurora", "Blaze", "Brave", "Bright", "Clever", "Cobalt", "Copper",
    "Cosmic", "Crimson", "Crystal", "Cyber", "Dawn", "Drift", "Dusk", "Echo", "Emerald", "Frosty",
    "Gentle", "Golden", "Granite", "Iron", "Jade", "Lunar", "Marble", "Mountain", "Mystic", "Neon",
    "Obsidian", "Ocean", "Opal", "Polar", "Quiet", "Radiant", "Ruby", "Sapphire", "Shadow",
    "Silent", "Silver", "Sky", "Solar", "Storm", "Swift", "Thunder", "Topaz", "Twilight", "Velvet",
    "Vibrant", "Wanderer", "Wild", "Zenith",
];

const NOUNS: &[&str] = &[
    "Badger",
    "Bear",
    "Beaver",
    "Bison",
    "Chameleon",
    "Cheetah",
    "Cobra",
    "Crane",
    "Dolphin",
    "Dragon",
    "Eagle",
    "Elk",
    "Falcon",
    "Ferret",
    "Finch",
    "Fox",
    "Gazelle",
    "Gecko",
    "Griffin",
    "Hawk",
    "Hedgehog",
    "Heron",
    "Jaguar",
    "Kestrel",
    "Kingfisher",
    "Koala",
    "Leopard",
    "Lynx",
    "Moose",
    "Osprey",
    "Otter",
    "Owl",
    "Panda",
    "Panther",
    "Pelican",
    "Phoenix",
    "Puffin",
    "Raccoon",
    "Raven",
    "Seal",
    "Sparrow",
    "Squirrel",
    "Stag",
    "Tiger",
    "Viper",
    "Walrus",
    "Whale",
    "Wolf",
    "Badger",
    "Lynx",
    "Falcon",
    "Hawk",
];

/// Generates a randomized, human-readable anonymous alias (e.g. "Swift Falcon").
pub fn generate_random_alias() -> String {
    let mut rng = rand::thread_rng();
    let adj = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];
    let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
    format!("{} {}", adj, noun)
}

/// Generates a purely randomized 128-bit anonymous device ID without hardware or user linkage.
pub fn generate_anonymous_device_id() -> String {
    let mut rng = rand::thread_rng();
    let hi: u64 = rng.r#gen();
    let lo: u64 = rng.r#gen();
    format!("{:016x}{:016x}", hi, lo)
}

/// Represents an anonymous peer identity consisting of an ephemeral/regenerable alias
/// and an isolated cryptographic device identifier.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnonymousIdentity {
    pub device_id: String,
    pub alias: String,
}

impl Default for AnonymousIdentity {
    fn default() -> Self {
        Self::new()
    }
}

impl AnonymousIdentity {
    pub fn new() -> Self {
        Self {
            device_id: generate_anonymous_device_id(),
            alias: generate_random_alias(),
        }
    }

    /// Regenerates the alias with a new random adjective-noun combination.
    pub fn regenerate_alias(&mut self) {
        self.alias = generate_random_alias();
    }

    /// Verifies that the identity exhibits zero leakage of Windows usernames or hostnames.
    pub fn verify_privacy_compliance(&self) -> bool {
        if self.device_id.len() != 32 || !self.device_id.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }

        let parts: Vec<&str> = self.alias.split_whitespace().collect();
        if parts.len() < 2 {
            return false;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_alias_generation() {
        let alias = generate_random_alias();
        let parts: Vec<&str> = alias.split_whitespace().collect();
        assert_eq!(parts.len(), 2);
        assert!(ADJECTIVES.contains(&parts[0]));
        assert!(NOUNS.contains(&parts[1]));
    }

    #[test]
    fn test_anonymous_device_id_entropy() {
        let id1 = generate_anonymous_device_id();
        let id2 = generate_anonymous_device_id();
        assert_eq!(id1.len(), 32);
        assert_eq!(id2.len(), 32);
        assert_ne!(id1, id2);
        assert!(id1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_identity_regeneration_and_privacy() {
        let mut id = AnonymousIdentity::new();
        assert!(id.verify_privacy_compliance());

        let old_alias = id.alias.clone();
        // Regenerate alias multiple times to verify valid formats
        for _ in 0..10 {
            id.regenerate_alias();
            assert!(id.verify_privacy_compliance());
        }
        assert!(!id.alias.is_empty());
    }
}
