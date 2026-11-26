use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub trait LockOrRecover<T: ?Sized> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> LockOrRecover<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub trait ReadOrRecover<T: ?Sized> {
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T>;
    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> ReadOrRecover<T> for RwLock<T> {
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn poison_mutex(shared: &Arc<Mutex<Vec<u32>>>) {
        let held = Arc::clone(shared);
        let crashed = std::thread::spawn(move || {
            let mut list = held.lock().unwrap();
            list.push(2);
            panic!("a worker panics while holding the lock");
        })
        .join();
        assert!(crashed.is_err());
    }

    fn poison_rwlock(shared: &Arc<RwLock<u32>>) {
        let held = Arc::clone(shared);
        let crashed = std::thread::spawn(move || {
            let mut value = held.write().unwrap();
            *value = 7;
            panic!("a worker panics while writing");
        })
        .join();
        assert!(crashed.is_err());
    }

    #[test]
    fn a_poisoned_mutex_still_hands_out_its_data() {
        let shared = Arc::new(Mutex::new(vec![1]));
        poison_mutex(&shared);
        assert!(shared.is_poisoned());
        assert_eq!(*shared.lock_or_recover(), vec![1, 2]);
        shared.lock_or_recover().push(3);
        assert_eq!(*shared.lock_or_recover(), vec![1, 2, 3]);
    }

    #[test]
    fn a_healthy_mutex_behaves_like_lock() {
        let shared = Mutex::new(5);
        *shared.lock_or_recover() += 1;
        assert_eq!(*shared.lock().unwrap(), 6);
        assert!(!shared.is_poisoned());
    }

    #[test]
    fn a_poisoned_rwlock_still_reads_and_writes() {
        let shared = Arc::new(RwLock::new(0));
        poison_rwlock(&shared);
        assert!(shared.is_poisoned());
        assert_eq!(*shared.read_or_recover(), 7);
        *shared.write_or_recover() = 9;
        assert_eq!(*shared.read_or_recover(), 9);
    }
}
