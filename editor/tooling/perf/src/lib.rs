pub use consts::*;

pub mod consts {
    pub const SUF_NORMAL: &str = "_pf_test";
    pub const SUF_MDATA: &str = "_pf_meta";
    pub const WEIGHT_DEFAULT: i32 = 50;
    pub const MDATA_LINE_PREF: &str = "!#!";
    pub const ITER_COUNT_LINE_NAME: &str = "iterations";
    pub const WEIGHT_LINE_NAME: &str = "weight";
    pub const IMPORTANCE_LINE_NAME: &str = "importance";
    pub const VERSION_LINE_NAME: &str = "version";
    pub const MDATA_VER: i32 = 1;
    pub const ITER_ENV_VAR: &str = "PERF_ITER";

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum Importance {
        Critical,
        Important,
        #[default]
        Average,
        Iffy,
        Fluff,
    }

    impl std::fmt::Display for Importance {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Critical => write!(f, "critical"),
                Self::Important => write!(f, "important"),
                Self::Average => write!(f, "average"),
                Self::Iffy => write!(f, "iffy"),
                Self::Fluff => write!(f, "fluff"),
            }
        }
    }
}
