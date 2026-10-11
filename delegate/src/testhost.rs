//! The tests' HOST: the delegate's secrets as a map.
use craftworks_identity::Host;
use std::collections::HashMap;

#[derive(Default)]
pub struct Map(pub HashMap<Vec<u8>, Vec<u8>>);
impl Host for Map {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.get(key).cloned()
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        self.0.insert(key.to_vec(), value.to_vec());
        true
    }
}
