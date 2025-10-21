#[repr(C)]
struct QVariantHeader {
    type_id: i32,
    flags: u32,  // is_shared, is_null и другие флаги
}

#[repr(C)]
union QVariantData {
    bool_val: i32,
    int_val: i32,
    double_val: f64,
    // другие типы...
}

#[repr(C)]
pub struct QVariant {
    header: QVariantHeader,
    data: QVariantData,
}

impl QVariant {
    pub fn from_bool(value: bool) -> Self {
        Self {
            header: QVariantHeader {
                type_id: 1, // QMetaType::Bool
                flags: 0,   // not shared, not null
            },
            data: QVariantData {
                bool_val: if value { 1 } else { 0 },
            },
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self as *const Self as *const u8,
                std::mem::size_of::<Self>()
            )
        }
    }
}
