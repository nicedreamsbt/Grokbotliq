// WARN: You can set anything here, including a discrim that's technically "wrong" for the struct
//   with that name, and prod will use that hash anyways. Don't change these hashes once a struct is
//   live in prod.
pub mod discriminators {
    pub const GROUP: [u8; 8] = [182, 23, 173, 240, 151, 206, 182, 67];
    pub const BANK: [u8; 8] = [142, 49, 166, 242, 50, 66, 97, 188];
    pub const ACCOUNT: [u8; 8] = [67, 178, 130, 109, 126, 114, 28, 42];
    pub const FEE_STATE: [u8; 8] = [63, 224, 16, 85, 193, 36, 235, 220];
    pub const STAKED_SETTINGS: [u8; 8] = [157, 140, 6, 77, 89, 173, 173, 125];
    pub const LIQUIDATION_RECORD: [u8; 8] = [95, 116, 23, 132, 89, 210, 245, 162];
    pub const ORDER: [u8; 8] = [134, 173, 223, 185, 77, 86, 28, 51];
    pub const EXECUTE_ORDER_RECORD: [u8; 8] = [6, 100, 107, 60, 164, 226, 56, 97];
    pub const BANK_METADATA: [u8; 8] = [49, 207, 31, 34, 67, 225, 169, 186];
    pub const SAME_ASSET_EMODE_REGISTRY: [u8; 8] = [222, 21, 195, 149, 193, 72, 219, 31];
}

pub mod ix_discriminators {
    pub const INIT_LIQUIDATION_RECORD: [u8; 8] = [236, 213, 238, 126, 147, 251, 164, 8];
    pub const START_LIQUIDATION: [u8; 8] = [244, 93, 90, 214, 192, 166, 191, 21];
    pub const END_LIQUIDATION: [u8; 8] = [110, 11, 244, 54, 229, 181, 22, 184];
    pub const START_EXECUTE_ORDER: [u8; 8] = [1, 70, 140, 134, 183, 29, 208, 224];
    pub const END_EXECUTE_ORDER: [u8; 8] = [115, 42, 20, 93, 121, 84, 178, 83];
    pub const LENDING_ACCOUNT_WITHDRAW: [u8; 8] = [36, 72, 74, 19, 210, 210, 192, 192];
    pub const LENDING_ACCOUNT_REPAY: [u8; 8] = [79, 209, 172, 177, 222, 51, 173, 151];
    pub const KAMINO_WITHDRAW: [u8; 8] = [199, 101, 41, 45, 213, 98, 224, 200];
    pub const DRIFT_WITHDRAW: [u8; 8] = [86, 59, 186, 123, 183, 181, 234, 137];
    pub const JUPLEND_WITHDRAW: [u8; 8] = [245, 164, 253, 202, 53, 77, 251, 221];
    pub const START_FLASHLOAN: [u8; 8] = [14, 131, 33, 220, 81, 186, 180, 107];
    pub const END_FLASHLOAN: [u8; 8] = [105, 124, 201, 106, 153, 2, 8, 156];
    pub const START_DELEVERAGE: [u8; 8] = [10, 138, 10, 57, 40, 232, 182, 193];
    pub const END_DELEVERAGE: [u8; 8] = [114, 14, 250, 143, 252, 104, 214, 209];
}
