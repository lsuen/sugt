/// 编译期产品线：功能版 vs 商店版（与 trial/self 授权维度正交）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductLine {
    Feature,
    Store,
}

pub fn product_line() -> ProductLine {
    match option_env!("SUGT_PRODUCT") {
        Some("store") => ProductLine::Store,
        _ => ProductLine::Feature,
    }
}

pub fn product_line_id() -> &'static str {
    match product_line() {
        ProductLine::Feature => "feature",
        ProductLine::Store => "store",
    }
}

pub fn product_line_label() -> &'static str {
    match product_line() {
        ProductLine::Feature => "功能版",
        ProductLine::Store => "全功能版",
    }
}

pub fn is_store_edition() -> bool {
    product_line() == ProductLine::Store
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_line_matches_compile_env() {
        match option_env!("SUGT_PRODUCT") {
            Some("store") => {
                assert_eq!(product_line(), ProductLine::Store);
                assert_eq!(product_line_id(), "store");
            }
            _ => {
                assert_eq!(product_line(), ProductLine::Feature);
                assert_eq!(product_line_id(), "feature");
            }
        }
    }
}
