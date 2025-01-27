use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

use anyhow::{anyhow, Result};
use base64::Engine;
use zeroize::Zeroize;

use rosenpass::protocol::{SPk, SSk};
use rosenpass_cipher_traits::Kem;
use rosenpass_ciphers::kem::StaticKem;
use rosenpass_secret_memory::{file::StoreSecret as _, Secret};

#[cfg(not(target_family = "unix"))]
pub fn genkey(_: &Path) -> Result<()> {
    Err(anyhow!(
        "Your system {} is not yet supported. We are happy to receive patches to address this :)",
        std::env::consts::OS
    ))
}

#[cfg(target_family = "unix")]
pub fn genkey(private_keys_dir: &Path) -> Result<()> {
    if private_keys_dir.exists() {
        if fs::metadata(private_keys_dir)?.permissions().mode() != 0o700 {
            return Err(anyhow!(
                "Directory {:?} has incorrect permissions: please use 0700 for proper security.",
                private_keys_dir
            ));
        }
    } else {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(private_keys_dir)?;
    }

    let wgsk_path = private_keys_dir.join("wgsk");
    let pqsk_path = private_keys_dir.join("pqsk");
    let pqpk_path = private_keys_dir.join("pqpk");

    if !wgsk_path.exists() {
        let wgsk: Secret<32> = Secret::random();

        let mut wgsk_file = OpenOptions::new()
            .write(true)
            .create(true)
            .mode(0o600)
            .open(wgsk_path)?;

        wgsk_file.write_all(
            base64::engine::general_purpose::STANDARD
                .encode(wgsk.secret())
                .as_bytes(),
        )?;
    } else {
        eprintln!(
            "WireGuard secret key already exists at {:#?}: not regenerating",
            wgsk_path
        );
    }

    if !pqsk_path.exists() && !pqpk_path.exists() {
        let mut pqsk = SSk::random();
        let mut pqpk = SPk::random();
        StaticKem::keygen(pqsk.secret_mut(), pqpk.secret_mut())?;
        pqpk.store_secret(pqpk_path)?;
        pqsk.store_secret(pqsk_path)?;
    } else {
        eprintln!(
            "Rosenpass keys already exist in {:#?}: not regenerating",
            private_keys_dir
        );
    }

    Ok(())
}

pub fn pubkey(private_keys_dir: &Path, public_keys_dir: &Path) -> Result<()> {
    if public_keys_dir.exists() {
        return Err(anyhow!("Directory {:?} already exists", public_keys_dir));
    }

    fs::create_dir_all(public_keys_dir)?;

    let private_wgsk = private_keys_dir.join("wgsk");
    let public_wgpk = public_keys_dir.join("wgpk");
    let private_pqpk = private_keys_dir.join("pqpk");
    let public_pqpk = public_keys_dir.join("pqpk");

    let wgsk = Secret::from_slice(
        &base64::engine::general_purpose::STANDARD.decode(fs::read_to_string(private_wgsk)?)?,
    );
    let mut wgpk: x25519_dalek::PublicKey = {
        let mut secret = x25519_dalek::StaticSecret::from(wgsk.secret().clone());
        let public = x25519_dalek::PublicKey::from(&secret);
        secret.zeroize();
        public
    };

    fs::write(
        public_wgpk,
        base64::engine::general_purpose::STANDARD.encode(wgpk.as_bytes()),
    )?;
    wgpk.zeroize();

    fs::copy(private_pqpk, public_pqpk)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use base64::Engine;
    use rosenpass::protocol::{SPk, SSk};
    use rosenpass_util::file::LoadValue;
    use tempfile::tempdir;

    use crate::key::{genkey, pubkey};

    #[test]
    fn it_works() {
        let private_keys_dir = tempdir().unwrap();
        fs::remove_dir(private_keys_dir.path()).unwrap();

        // Guranteed to have 16MB of stack size
        stacker::grow(8 * 1024 * 1024, || {
            assert!(genkey(private_keys_dir.path()).is_ok());
        });

        assert!(private_keys_dir.path().exists());
        assert!(private_keys_dir.path().is_dir());
        assert!(SPk::load(private_keys_dir.path().join("pqpk")).is_ok());
        assert!(SSk::load(private_keys_dir.path().join("pqsk")).is_ok());
        assert!(base64::engine::general_purpose::STANDARD
            .decode(&fs::read_to_string(private_keys_dir.path().join("wgsk")).unwrap())
            .is_ok());

        let public_keys_dir = tempdir().unwrap();
        fs::remove_dir(public_keys_dir.path()).unwrap();

        // Guranteed to have 16MB of stack size
        stacker::grow(8 * 1024 * 1024, || {
            assert!(pubkey(private_keys_dir.path(), public_keys_dir.path()).is_ok());
        });

        assert!(public_keys_dir.path().exists());
        assert!(public_keys_dir.path().is_dir());
        assert!(SPk::load(public_keys_dir.path().join("pqpk")).is_ok());
        assert!(base64::engine::general_purpose::STANDARD
            .decode(&fs::read_to_string(public_keys_dir.path().join("wgpk")).unwrap())
            .is_ok());

        let pk_1 = fs::read(private_keys_dir.path().join("pqpk")).unwrap();
        let pk_2 = fs::read(public_keys_dir.path().join("pqpk")).unwrap();
        assert_eq!(pk_1, pk_2);
    }
}
