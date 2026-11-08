//! Transactional changes to per-process audio policies.

use std::collections::BTreeSet;
use std::fmt;

pub trait PolicyStore {
    type Value: Clone + PartialEq;
    type Error: fmt::Display;

    fn read(&mut self, pid: u32, role: i32) -> Result<Self::Value, Self::Error>;
    fn write(&mut self, pid: u32, role: i32, value: &Self::Value) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub struct RoutingError {
    pub cause: String,
    pub rollback_errors: Vec<String>,
}

impl fmt::Display for RoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.cause)?;
        for error in &self.rollback_errors {
            write!(f, "; rollback: {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for RoutingError {}

/// Applies a target to both render roles for all PIDs, restoring snapshots on failure.
/// A failed rollback is reported explicitly: process or audio-service changes can
/// prevent a transaction from being atomic across Windows API calls.
pub fn route<S: PolicyStore>(
    store: &mut S,
    pids: &[u32],
    target: &S::Value,
) -> Result<(), RoutingError> {
    let unique: BTreeSet<_> = pids.iter().copied().collect();
    if unique.is_empty() || unique.contains(&0) {
        return Err(RoutingError {
            cause: "At least one nonzero process ID is required".into(),
            rollback_errors: vec![],
        });
    }
    let mut snapshots = Vec::new();
    for pid in unique {
        for role in 0..2 {
            let old = store.read(pid, role).map_err(|e| RoutingError {
                cause: format!("Could not read PID {pid}, role {role}: {e}"),
                rollback_errors: vec![],
            })?;
            snapshots.push((pid, role, old));
        }
    }
    let mut attempted = 0;
    for (pid, role, _) in &snapshots {
        attempted += 1; // A failed write may still have changed the policy.
        if let Err(e) = store.write(*pid, *role, target) {
            return Err(rollback(
                store,
                &snapshots[..attempted],
                format!("Could not set PID {pid}, role {role}: {e}"),
            ));
        }
    }
    for (pid, role, _) in &snapshots {
        match store.read(*pid, *role) {
            Ok(value) if value == *target => (),
            Ok(_) => {
                return Err(rollback(
                    store,
                    &snapshots,
                    format!("Verification failed for PID {pid}, role {role}"),
                ))
            }
            Err(e) => {
                return Err(rollback(
                    store,
                    &snapshots,
                    format!("Could not verify PID {pid}, role {role}: {e}"),
                ))
            }
        }
    }
    Ok(())
}

fn rollback<S: PolicyStore>(
    store: &mut S,
    snapshots: &[(u32, i32, S::Value)],
    cause: String,
) -> RoutingError {
    let mut errors = Vec::new();
    for (pid, role, old) in snapshots.iter().rev() {
        if let Err(e) = store.write(*pid, *role, old) {
            errors.push(format!("PID {pid}, role {role}: {e}"));
        }
    }
    for (pid, role, old) in snapshots {
        match store.read(*pid, *role) {
            Ok(value) if value == *old => (),
            Ok(_) => errors.push(format!(
                "PID {pid}, role {role}: restored value did not match"
            )),
            Err(e) => errors.push(format!("PID {pid}, role {role}: could not verify: {e}")),
        }
    }
    RoutingError {
        cause,
        rollback_errors: errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Fake {
        values: BTreeMap<(u32, i32), u8>,
        writes: usize,
        fail_writes: Vec<usize>,
        fail_read: Option<(u32, i32)>,
    }
    impl PolicyStore for Fake {
        type Value = u8;
        type Error = &'static str;
        fn read(&mut self, pid: u32, role: i32) -> Result<u8, Self::Error> {
            if self.fail_read == Some((pid, role)) {
                return Err("read failed");
            }
            Ok(*self.values.get(&(pid, role)).unwrap_or(&0))
        }
        fn write(&mut self, pid: u32, role: i32, value: &u8) -> Result<(), Self::Error> {
            self.writes += 1;
            self.values.insert((pid, role), *value);
            if self.fail_writes.contains(&self.writes) {
                Err("write failed")
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn routes_all_unique_processes_and_roles() {
        let mut store = Fake::default();
        route(&mut store, &[7, 3, 7], &9).unwrap();
        assert_eq!(store.writes, 4);
        assert!([3, 7]
            .iter()
            .all(|pid| (0..2).all(|role| store.values[&(*pid, role)] == 9)));
    }
    #[test]
    fn read_failure_never_changes_any_policy() {
        let mut store = Fake {
            fail_read: Some((7, 0)),
            ..Default::default()
        };
        assert!(route(&mut store, &[3, 7], &9).is_err());
        assert_eq!(store.writes, 0);
    }
    #[test]
    fn failed_write_is_also_rolled_back() {
        let mut store = Fake {
            fail_writes: vec![3],
            ..Default::default()
        };
        let error = route(&mut store, &[3, 7], &9).unwrap_err();
        assert!(error.rollback_errors.is_empty());
        assert!(store.values.values().all(|value| *value == 0));
    }
    #[test]
    fn failed_rollback_is_reported_and_other_roles_are_restored() {
        let mut store = Fake {
            fail_writes: vec![3, 4],
            ..Default::default()
        };
        let error = route(&mut store, &[3, 7], &9).unwrap_err();
        assert_eq!(error.rollback_errors.len(), 1);
        assert!(store.values.values().all(|value| *value == 0));
    }
    #[test]
    fn empty_or_zero_pids_are_rejected() {
        let mut store = Fake::default();
        assert!(route(&mut store, &[], &9).is_err());
        assert!(route(&mut store, &[0, 3], &9).is_err());
        assert_eq!(store.writes, 0);
    }
}
