use super::{DataAccess, INTEGER, OPTIONAL_TEXT, Spec, TEXT};

pub(super) const SPECS: &[Spec] = &[
    Spec {
        key: "header",
        sha256: "07021caacf7f24372475a9f7f5629272b97b3b1b66cc96029b6e854ee484efbe",
        access: DataAccess::Read,
        parameters: &[TEXT],
        columns: &[TEXT, TEXT, TEXT, INTEGER, INTEGER, INTEGER, TEXT, TEXT],
    },
    Spec {
        key: "results",
        sha256: "e6c428f6050949602a81d9f1d1fe5fd9efab569fb3f63cc8d5554f8df9380f05",
        access: DataAccess::Read,
        parameters: &[TEXT],
        columns: &[TEXT, TEXT, TEXT, OPTIONAL_TEXT, OPTIONAL_TEXT],
    },
    Spec {
        key: "create-header",
        sha256: "ccdb3f494801d19bc6b34f1ffeaba6e9efccdd6f9be86e439617f45c85eee492",
        access: DataAccess::Write,
        parameters: &[TEXT, TEXT, INTEGER],
        columns: &[],
    },
    Spec {
        key: "insert-pending",
        sha256: "ae491ba690baef5c7714f38ab7dd9bcefe74376baadffd325c38f7f2f4b451ec",
        access: DataAccess::Write,
        parameters: &[TEXT, TEXT, TEXT, TEXT],
        columns: &[],
    },
    Spec {
        key: "counts",
        sha256: "34093d953a0ca9a92c5f7059cf635c4560cc6185942fabcdefce0d9e6937a4d6",
        access: DataAccess::Read,
        parameters: &[TEXT],
        columns: &[INTEGER, INTEGER],
    },
    Spec {
        key: "finish",
        sha256: "66a0856e2fc0bdead2d57c2b76eb7436a54eabf2a788d1a6297d14710ee000f3",
        access: DataAccess::Write,
        parameters: &[TEXT, TEXT],
        columns: &[],
    },
    Spec {
        key: "result",
        sha256: "0cd616c049ae2dd49c4c6c4209b299e405c0ea2416e4335e349eadfb779fc40e",
        access: DataAccess::Write,
        parameters: &[TEXT, TEXT, OPTIONAL_TEXT, OPTIONAL_TEXT],
        columns: &[],
    },
    Spec {
        key: "list-ids",
        sha256: "dce5599730a1ba6c30b497f0f80402bc4ba3382f3e139a5f9015b3ee82d13dab",
        access: DataAccess::Read,
        parameters: &[],
        columns: &[TEXT],
    },
    Spec {
        key: "cancel",
        sha256: "ecda0b0202c4f5fa4d03cbc78ca6a302058ce513b0a1145b39fe5b0aa431f49b",
        access: DataAccess::Write,
        parameters: &[TEXT],
        columns: &[],
    },
];
