//! Read-only preflight before starting a mainnet wallet without Lightning.
//!
//! LDK does not expose a side-effect-free inspector for its persisted manager or sweeper.
//! Their presence is therefore ambiguous, including snapshots from an old empty node. Do not
//! deserialize them, infer safety from absent monitors, or delete them to make unlock succeed.

use std::path::Path;

use lightning::rgb_utils::{RGB_PRIMARY_NS, RGB_WALLET_CONFIG_NS};
use sea_orm::{DatabaseConnection, EntityTrait, QuerySelect};

use crate::database::entities::{ChannelPeerEntity, KvStoreColumn, KvStoreEntity};
use crate::error::APIError;

// These are the existing, reconstructible mirrors written by ldk::save_config. This is an
// exact allowlist, not permission to replay arbitrary data under the wallet_config namespace.
const COMMON_CONFIG_KEYS: [&str; 6] = [
    "indexer_url",
    "bitcoin_network",
    "wallet_fingerprint",
    "wallet_account_xpub_vanilla",
    "wallet_account_xpub_colored",
    "wallet_master_fingerprint",
];

// Keep recognizing this persisted namespace even in a build without the vss feature.
const PENDING_NAMESPACE: &str = "vss_pending";

fn needs_review(detail: impl Into<String>) -> APIError {
    APIError::MainnetLightningState(detail.into())
}

fn is_common_config(primary: &str, secondary: &str, key: &str) -> bool {
    primary == RGB_PRIMARY_NS
        && secondary == RGB_WALLET_CONFIG_NS
        && COMMON_CONFIG_KEYS.contains(&key)
}

fn is_common_remote_key(key: &str) -> bool {
    COMMON_CONFIG_KEYS
        .iter()
        .any(|name| key == format!("{RGB_PRIMARY_NS}/{RGB_WALLET_CONFIG_NS}/{name}"))
}

/// Render only bounded key metadata, never a stored value or full filesystem path.
fn key_label(primary: &str, secondary: &str, key: &str) -> String {
    fn bounded(value: &str) -> String {
        let prefix: String = value.chars().take(64).collect();
        format!("{prefix:?}")
    }
    format!(
        "{}/{}/{}",
        bounded(primary),
        bounded(secondary),
        bounded(key)
    )
}

fn check_pending_intent(key: &str, value: &[u8]) -> Result<(), APIError> {
    // SyncedKvStore stores a one-byte tag followed by a put payload, or just tag 0 for a
    // removal. It otherwise loads every intent and may drain it during an unrelated write.
    let well_formed = match value.split_first() {
        Some((0, payload)) => payload.is_empty(),
        Some((1, payload)) => std::str::from_utf8(payload).is_ok(),
        _ => false,
    };
    if !is_common_remote_key(key) || !well_formed {
        return Err(needs_review(format!(
            "local pending replication intent {} cannot be replayed by a wallet-only node",
            key_label(PENDING_NAMESPACE, "", key)
        )));
    }
    Ok(())
}

fn check_ldk_directory(ldk_data_dir: &Path) -> Result<(), APIError> {
    let metadata = match std::fs::symlink_metadata(ldk_data_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(needs_review(
                "local Lightning directory cannot be inspected",
            ))
        }
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(needs_review(
            "local Lightning directory is not a regular directory",
        ));
    }
    let entries = std::fs::read_dir(ldk_data_dir)
        .map_err(|_| needs_review("local Lightning directory cannot be enumerated"))?;
    for entry in entries {
        let entry =
            entry.map_err(|_| needs_review("local Lightning directory entry is unreadable"))?;
        let file_type = entry
            .file_type()
            .map_err(|_| needs_review("local Lightning directory entry cannot be inspected"))?;
        // The shared logger creates this directory even before the first wallet unlock.
        if entry.file_name() == crate::utils::LOGS_DIR && file_type.is_dir() {
            continue;
        }
        return Err(needs_review(format!(
            "local Lightning directory contains an unclassified entry {}",
            key_label("", "", &entry.file_name().to_string_lossy())
        )));
    }
    Ok(())
}

#[cfg(feature = "vss")]
fn check_remote_keys(keys: &[String]) -> Result<(), APIError> {
    for key in keys {
        if !is_common_remote_key(key) {
            return Err(needs_review(format!(
                "remote Lightning store contains an unclassified key {}",
                key_label("", "", key)
            )));
        }
    }
    Ok(())
}

/// Caller must serialize startup, acquire any configured VSS fence first, and run this on a
/// blocking worker before constructing SyncedKvStore or writing configuration mirrors. This
/// deliberately refuses opaque legacy snapshots; it does not claim they contain live funds.
pub(crate) fn check_mainnet_legacy_state(
    database: &DatabaseConnection,
    ldk_data_dir: &Path,
    #[cfg(feature = "vss")] remote: Option<&crate::vss_kv_store::VssKvStore>,
) -> Result<(), APIError> {
    // Select keys first: refusing an opaque manager must not load or decode its payload.
    let keys: Vec<(String, String, String)> = crate::runtime::block_on(
        KvStoreEntity::find()
            .select_only()
            .columns([
                KvStoreColumn::PrimaryNamespace,
                KvStoreColumn::SecondaryNamespace,
                KvStoreColumn::Key,
            ])
            .into_tuple()
            .all(database),
    )
    .map_err(|_| needs_review("local Lightning key inventory cannot be read"))?;
    for (primary, secondary, key) in keys {
        if is_common_config(&primary, &secondary, &key) {
            continue;
        }
        if primary == PENDING_NAMESPACE && secondary.is_empty() && is_common_remote_key(&key) {
            let row = crate::runtime::block_on(
                KvStoreEntity::find_by_id((primary, secondary, key.clone())).one(database),
            )
            .map_err(|_| needs_review("local pending replication intent cannot be read"))?
            .ok_or_else(|| {
                needs_review("local pending replication inventory changed during startup")
            })?;
            check_pending_intent(&key, &row.value)?;
            continue;
        }
        return Err(needs_review(format!(
            "local Lightning store contains an unclassified key {}",
            key_label(&primary, &secondary, &key)
        )));
    }

    if crate::runtime::block_on(ChannelPeerEntity::find().one(database))
        .map_err(|_| needs_review("local Lightning peer inventory cannot be read"))?
        .is_some()
    {
        return Err(needs_review("local Lightning peer history requires review"));
    }
    check_ldk_directory(ldk_data_dir)?;

    #[cfg(feature = "vss")]
    if let Some(remote) = remote {
        // list_all_keys excludes the ownership fence and paginates the complete raw inventory.
        // No restore, pending-intent cleanup, put or delete is performed here.
        let keys = remote
            .list_all_keys()
            .map_err(|_| needs_review("remote Lightning key inventory cannot be read"))?;
        check_remote_keys(&keys)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::block_on;
    use sea_orm::{ActiveModelTrait, ActiveValue};

    // Keep setup, reads and preflight on the repository database runtime: SQLx schedules
    // pooled connection releases there, so another runtime cannot drive that queued work.
    fn inspect(database: &DatabaseConnection, ldk_data_dir: &Path) -> Result<(), APIError> {
        check_mainnet_legacy_state(
            database,
            ldk_data_dir,
            #[cfg(feature = "vss")]
            None,
        )
    }

    #[test]
    fn only_exact_common_config_locations_are_allowed() {
        for key in COMMON_CONFIG_KEYS {
            assert!(is_common_config("rgb", "wallet_config", key));
            assert!(is_common_remote_key(&format!("rgb/wallet_config/{key}")));
        }
        for (primary, secondary, key) in [
            ("", "", "manager"),
            ("", "", "output_sweeper"),
            ("monitors", "", "monitor"),
            ("monitor_updates", "channel", "1"),
            ("rgb", "wallet_config", "unknown"),
            ("rgb", "", "indexer_url"),
            ("rgb", "wallet_config", "indexer_url/manager"),
        ] {
            assert!(!is_common_config(primary, secondary, key));
        }
        assert!(!is_common_remote_key("rgb//wallet_config/indexer_url"));
    }

    #[test]
    fn pending_config_put_and_delete_are_allowed_but_protocol_or_malformed_intents_are_not() {
        let key = "rgb/wallet_config/indexer_url";
        assert!(check_pending_intent(key, b"\x01https://indexer.invalid").is_ok());
        assert!(check_pending_intent(key, &[0]).is_ok());
        for value in [&[][..], &[2][..], &[0, 1][..], &[1, 255][..]] {
            assert!(check_pending_intent(key, value).is_err());
        }
        for key in ["_/_/manager", "monitor_updates/channel/1", "malformed"] {
            assert!(check_pending_intent(key, &[0]).is_err());
            assert!(check_pending_intent(key, &[1]).is_err());
        }
    }

    #[test]
    fn shared_logs_are_allowed_but_unclassified_files_remain_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let ldk_dir = temp.path().join(".ldk");
        assert!(check_ldk_directory(&ldk_dir).is_ok());
        std::fs::create_dir_all(ldk_dir.join(crate::utils::LOGS_DIR)).unwrap();
        std::fs::write(
            ldk_dir.join(crate::utils::LOGS_DIR).join("logs.txt"),
            b"log",
        )
        .unwrap();
        assert!(check_ldk_directory(&ldk_dir).is_ok());
        let state_path = ldk_dir.join("funding_consignment");
        std::fs::write(&state_path, b"protected bytes").unwrap();
        assert!(check_ldk_directory(&ldk_dir).is_err());
        assert_eq!(std::fs::read(state_path).unwrap(), b"protected bytes");
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_is_not_treated_as_the_shared_log_directory() {
        let temp = tempfile::tempdir().unwrap();
        let ldk_dir = temp.path().join(".ldk");
        std::fs::create_dir(&ldk_dir).unwrap();
        std::os::unix::fs::symlink(temp.path(), ldk_dir.join(crate::utils::LOGS_DIR)).unwrap();
        assert!(check_ldk_directory(&ldk_dir).is_err());
    }

    #[cfg(feature = "vss")]
    #[test]
    fn remote_only_snapshots_and_noncanonical_keys_are_rejected_without_decoding() {
        assert!(check_remote_keys(&[]).is_ok());
        assert!(check_remote_keys(&["rgb/wallet_config/bitcoin_network".into()]).is_ok());
        for key in [
            "_/_/manager",
            "_/_/output_sweeper",
            "monitor_updates/channel/1",
            "rgb/pending_funding/id",
            "vss_pending/_/rgb/wallet_config/indexer_url",
            "rgb/wallet_config/bitcoin_network/extra",
            "malformed",
        ] {
            assert!(check_remote_keys(&[key.into()]).is_err());
        }
    }

    #[test]
    fn diagnostic_keys_are_bounded_and_escape_control_characters() {
        let label = key_label("", "", &format!("\n{}", "x".repeat(4096)));
        assert!(!label.contains('\n'));
        assert!(label.len() < 100);
    }

    #[test]
    fn database_preflight_accepts_fresh_wallet_and_preserves_ambiguous_snapshots() {
        use rln_migration::{Migrator, MigratorTrait};

        let temp = tempfile::tempdir().unwrap();
        let database = block_on(crate::utils::open_database_pool(temp.path())).unwrap();
        block_on(Migrator::up(&database, None)).unwrap();
        let ldk_data_dir = temp.path().join(".ldk");
        inspect(&database, &ldk_data_dir).expect("fresh wallet has no legacy state");

        for key in COMMON_CONFIG_KEYS {
            block_on(
                crate::database::entities::KvStoreActMod {
                    primary_namespace: ActiveValue::Set("rgb".into()),
                    secondary_namespace: ActiveValue::Set("wallet_config".into()),
                    key: ActiveValue::Set(key.into()),
                    value: ActiveValue::Set(b"common value".to_vec()),
                }
                .insert(&database),
            )
            .unwrap();
        }
        inspect(&database, &ldk_data_dir).expect("common configuration is allowed");

        block_on(
            crate::database::entities::KvStoreActMod {
                primary_namespace: ActiveValue::Set(String::new()),
                secondary_namespace: ActiveValue::Set(String::new()),
                key: ActiveValue::Set("manager".into()),
                value: ActiveValue::Set(b"opaque legacy snapshot".to_vec()),
            }
            .insert(&database),
        )
        .unwrap();
        let before = block_on(KvStoreEntity::find().all(&database)).unwrap();
        for _ in 0..2 {
            let result = inspect(&database, &ldk_data_dir);
            assert!(
                matches!(&result, Err(APIError::MainnetLightningState(detail)) if detail.contains("unclassified key") && detail.contains("manager")),
                "{result:?}"
            );
            assert_eq!(
                block_on(KvStoreEntity::find().all(&database)).unwrap(),
                before
            );
        }
        block_on(database.close()).unwrap();
    }

    #[test]
    fn database_preflight_never_cleans_or_replays_pending_intents() {
        use rln_migration::{Migrator, MigratorTrait};

        let temp = tempfile::tempdir().unwrap();
        let database = block_on(crate::utils::open_database_pool(temp.path())).unwrap();
        block_on(Migrator::up(&database, None)).unwrap();
        for (primary, secondary, key, value, accepted) in [
            (
                PENDING_NAMESPACE,
                "",
                "rgb/wallet_config/indexer_url",
                &b"\x01https://indexer.invalid"[..],
                true,
            ),
            (
                PENDING_NAMESPACE,
                "",
                "rgb/wallet_config/indexer_url",
                &[0][..],
                true,
            ),
            (
                PENDING_NAMESPACE,
                "",
                "rgb/wallet_config/indexer_url",
                &[][..],
                false,
            ),
            (
                PENDING_NAMESPACE,
                "",
                "rgb/wallet_config/indexer_url",
                &[0, 1][..],
                false,
            ),
            (PENDING_NAMESPACE, "", "_/_/manager", &[0][..], false),
            (PENDING_NAMESPACE, "", "_/_/manager", &[1][..], false),
            ("monitor_updates", "channel", "1", &b"opaque"[..], false),
        ] {
            block_on(
                crate::database::entities::KvStoreActMod {
                    primary_namespace: ActiveValue::Set(primary.into()),
                    secondary_namespace: ActiveValue::Set(secondary.into()),
                    key: ActiveValue::Set(key.into()),
                    value: ActiveValue::Set(value.to_vec()),
                }
                .insert(&database),
            )
            .unwrap();
            let before = block_on(KvStoreEntity::find().all(&database)).unwrap();
            let result = inspect(&database, &temp.path().join(".ldk"));
            assert_eq!(
                result.is_ok(),
                accepted,
                "{primary}/{secondary}/{key}: {result:?}"
            );
            if !accepted {
                assert!(
                    matches!(&result, Err(APIError::MainnetLightningState(detail)) if detail.contains(key)),
                    "{result:?}"
                );
            }
            assert_eq!(
                block_on(KvStoreEntity::find().all(&database)).unwrap(),
                before
            );
            block_on(
                KvStoreEntity::delete_by_id((
                    primary.to_owned(),
                    secondary.to_owned(),
                    key.to_owned(),
                ))
                .exec(&database),
            )
            .unwrap();
        }
        block_on(database.close()).unwrap();
    }

    #[test]
    fn database_preflight_preserves_peer_history() {
        use rln_migration::{Migrator, MigratorTrait};

        let temp = tempfile::tempdir().unwrap();
        let database = block_on(crate::utils::open_database_pool(temp.path())).unwrap();
        block_on(Migrator::up(&database, None)).unwrap();
        block_on(
            crate::database::entities::ChannelPeerActMod {
                pubkey: ActiveValue::Set("legacy peer".into()),
                address: ActiveValue::Set("127.0.0.1:9735".into()),
                created_at: ActiveValue::Set(chrono::Utc::now()),
            }
            .insert(&database),
        )
        .unwrap();
        let before = block_on(ChannelPeerEntity::find().all(&database)).unwrap();
        let result = inspect(&database, &temp.path().join(".ldk"));
        assert!(
            matches!(&result, Err(APIError::MainnetLightningState(detail)) if detail == "local Lightning peer history requires review"),
            "{result:?}"
        );
        assert_eq!(
            block_on(ChannelPeerEntity::find().all(&database)).unwrap(),
            before
        );
        block_on(database.close()).unwrap();
    }
}
