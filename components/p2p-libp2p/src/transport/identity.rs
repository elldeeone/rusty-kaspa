use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use libp2p::identity::Keypair;
use libp2p::{PeerId, identity};
use tempfile::Builder;

use crate::config::Config;

use super::Libp2pError;

/// Libp2p identity wrapper (ed25519).
#[derive(Clone)]
pub struct Libp2pIdentity {
    pub keypair: Keypair,
    pub peer_id: PeerId,
    pub persisted_path: Option<PathBuf>,
}

impl Libp2pIdentity {
    pub fn from_config(config: &Config) -> Result<Self, Libp2pError> {
        match &config.identity {
            crate::Identity::Ephemeral => {
                let keypair = identity::Keypair::generate_ed25519();
                let peer_id = PeerId::from(keypair.public());
                Ok(Self { keypair, peer_id, persisted_path: None })
            }
            crate::Identity::Persisted(path) => {
                let keypair = load_or_generate_key(path).map_err(|e| Libp2pError::Identity(e.to_string()))?;
                let peer_id = PeerId::from(keypair.public());
                Ok(Self { keypair, peer_id, persisted_path: Some(path.clone()) })
            }
        }
    }

    pub fn peer_id_string(&self) -> String {
        self.peer_id.to_string()
    }
}

fn load_or_generate_key(path: &Path) -> io::Result<Keypair> {
    if let Some(keypair) = load_existing_key(path)? {
        return Ok(keypair);
    }

    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let keypair = identity::Keypair::generate_ed25519();
    let bytes = keypair.to_protobuf_encoding().map_err(map_identity_err)?;
    let mut temp = Builder::new().prefix(".libp2p-identity-").tempfile_in(parent)?;
    set_private_permissions(temp.as_file())?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;

    match temp.persist_noclobber(path) {
        Ok(file) => {
            file.sync_all()?;
            Ok(keypair)
        }
        Err(err) if err.error.kind() == io::ErrorKind::AlreadyExists => load_existing_key(path)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "libp2p identity appeared but could not be loaded")),
        Err(err) => Err(err.error),
    }
}

fn load_existing_key(path: &Path) -> io::Result<Option<Keypair>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "refusing to load libp2p identity through a symlink"));
    }
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "libp2p identity path is not a regular file"));
    }

    let mut file = fs::OpenOptions::new().read(true).open(path)?;
    let opened_metadata = file.metadata()?;
    ensure_same_file(&metadata, &opened_metadata)?;
    repair_private_permissions(&file, &opened_metadata)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Keypair::from_protobuf_encoding(&bytes).map(Some).map_err(map_identity_err)
}

#[cfg(unix)]
fn set_private_permissions(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_permissions(_file: &fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn repair_private_permissions(file: &fs::File, metadata: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & 0o077 != 0 {
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn repair_private_permissions(_file: &fs::File, _metadata: &fs::Metadata) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn ensure_same_file(expected: &fs::Metadata, opened: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    if expected.dev() != opened.dev() || expected.ino() != opened.ino() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "libp2p identity changed while it was being opened"));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_same_file(_expected: &fs::Metadata, _opened: &fs::Metadata) -> io::Result<()> {
    Ok(())
}

fn map_identity_err(err: impl ToString) -> io::Error {
    io::Error::other(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn persisted_identity_is_private_and_stable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("libp2p.id");

        let first = load_or_generate_key(&path).unwrap();
        let second = load_or_generate_key(&path).unwrap();

        assert_eq!(PeerId::from(first.public()), PeerId::from(second.public()));
        assert_private_permissions(&path);
    }

    #[cfg(unix)]
    #[test]
    fn persisted_identity_repairs_unsafe_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let path = dir.path().join("libp2p.id");
        let expected = identity::Keypair::generate_ed25519();
        fs::write(&path, expected.to_protobuf_encoding().unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let loaded = load_or_generate_key(&path).unwrap();

        assert_eq!(PeerId::from(expected.public()), PeerId::from(loaded.public()));
        assert_private_permissions(&path);
    }

    #[cfg(unix)]
    #[test]
    fn persisted_identity_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let target = dir.path().join("target.id");
        let link = dir.path().join("libp2p.id");
        let keypair = identity::Keypair::generate_ed25519();
        fs::write(&target, keypair.to_protobuf_encoding().unwrap()).unwrap();
        symlink(&target, &link).unwrap();

        let err = load_or_generate_key(&link).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("symlink"));
    }

    #[cfg(unix)]
    fn assert_private_permissions(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[cfg(not(unix))]
    fn assert_private_permissions(_path: &Path) {}
}
