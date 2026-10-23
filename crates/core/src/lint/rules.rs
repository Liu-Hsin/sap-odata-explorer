//! Lint rules grouped by category. Each `check_*` function pushes its
//! findings onto a shared `Vec<LintFinding>` in the order the parent
//! evaluator calls them; the in-function ordering is part of the
//! contract too — `tests::consistency_rules_emit_in_canonical_order`
//! and `tests::integrity_rules_emit_in_canonical_order` in
//! `lint/mod.rs` lock the stream order the CLI and desktop render.
//!
//! Behaviour-preserving move from `lint/mod.rs`: no rule changes, no
//! visibility widening past `pub(super)`, no public-API impact.

use crate::metadata::EntityType;

use super::profile::LintProfile;
use super::types::{LintCategory, LintFinding, LintSeverity, actionable, pass};

// ── Identity ────────────────────────────────────────────────────────
// Value-help entities don't need HeaderInfo — they're internal lookup
// targets. Everything else does.
pub(super) fn check_identity(et: &EntityType, profile: LintProfile, out: &mut Vec<LintFinding>) {
    let is_value_help = profile == LintProfile::ValueHelp;

    if et.header_info.is_some() {
        out.push(pass(
            LintCategory::Identity,
            "header_info",
            "已声明 UI.HeaderInfo。".to_string(),
        ));
    } else if !is_value_help {
        out.push(actionable(
            LintSeverity::Miss,
            LintCategory::Identity,
            "header_info",
            "未声明 UI.HeaderInfo——Fiori 将使用技术类型名称作为标题。".to_string(),
            "@UI.headerInfo",
            "用于对象页标题及列表报表的单数/复数名称。",
        ));
    }
    if !et.semantic_keys.is_empty() {
        out.push(pass(
            LintCategory::Identity,
            "semantic_key",
            format!("Common.SemanticKey：{}", et.semantic_keys.join(", ")),
        ));
    } else if et.keys.iter().any(|k| is_uuid_like_key(k)) {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Identity,
            "semantic_key",
            "技术键看起来像 UUID，但未声明 Common.SemanticKey——Fiori 将显示 UUID。".to_string(),
            "@ObjectModel.semanticKey",
            "让 Fiori 显示业务键（例如 Product、OrderID），而不是技术 UUID。",
        ));
    }
}

// ── List-report ─────────────────────────────────────────────────────
// LineItem only makes sense for list-report / analytical shapes.
// Value-help entities never need it; object-page roots defer it
// to their item-child entity.
pub(super) fn check_list_report(et: &EntityType, profile: LintProfile, out: &mut Vec<LintFinding>) {
    let is_value_help = profile == LintProfile::ValueHelp;
    let is_object_page = profile == LintProfile::ObjectPage;
    let is_analytical = profile == LintProfile::Analytical;

    if !et.line_item.is_empty() {
        out.push(pass(
            LintCategory::ListReport,
            "line_item",
            format!("UI.LineItem：已声明 {} 列。", et.line_item.len()),
        ));
    } else if !is_value_help && !is_object_page && !is_analytical {
        out.push(actionable(
            LintSeverity::Miss,
            LintCategory::ListReport,
            "line_item",
            "未声明 UI.LineItem——列表报表没有默认列。".to_string(),
            "@UI.lineItem",
            "声明列表报表默认显示哪些列及其顺序。",
        ));
    }
    if !et.request_at_least.is_empty() {
        out.push(pass(
            LintCategory::ListReport,
            "request_at_least",
            format!(
                "UI.PresentationVariant.RequestAtLeast：额外包含 {} 个辅助字段。",
                et.request_at_least.len(),
            ),
        ));
    }
    if !et.sort_order.is_empty() {
        out.push(pass(
            LintCategory::ListReport,
            "sort_order",
            format!(
                "UI.PresentationVariant.SortOrder：已声明 {} 条排序规则。",
                et.sort_order.len(),
            ),
        ));
    }
}

// ── Filtering ───────────────────────────────────────────────────────
// SelectionFields drive the filter bar — relevant for list-report
// and transactional-list services. Object-page and value-help
// don't surface a filter bar to the end user.
pub(super) fn check_filtering(et: &EntityType, profile: LintProfile, out: &mut Vec<LintFinding>) {
    let is_value_help = profile == LintProfile::ValueHelp;
    let is_object_page = profile == LintProfile::ObjectPage;

    if !et.selection_fields.is_empty() {
        out.push(pass(
            LintCategory::Filtering,
            "selection_fields",
            format!(
                "UI.SelectionFields：已声明 {} 个字段。",
                et.selection_fields.len()
            ),
        ));
    } else if !is_value_help && !is_object_page {
        out.push(actionable(
            LintSeverity::Miss,
            LintCategory::Filtering,
            "selection_fields",
            "未声明 UI.SelectionFields——筛选栏默认为空。".to_string(),
            "@UI.selectionField",
            "填充 Fiori 筛选栏，指定用户可直接使用的筛选字段。",
        ));
    }
    if !et.selection_variants.is_empty() {
        out.push(pass(
            LintCategory::Filtering,
            "selection_variants",
            format!(
                "UI.SelectionVariant：已声明 {} 个筛选变体。",
                et.selection_variants.len(),
            ),
        ));
    }
}

// ── Field-level consistency ─────────────────────────────────────────
// Decimal-shaped properties without a unit / currency companion;
// code-shaped properties without a paired Common.Text.
pub(super) fn check_fields(et: &EntityType, out: &mut Vec<LintFinding>) {
    let decimal_without_unit: Vec<&str> = et
        .properties
        .iter()
        .filter(|p| {
            looks_monetary_or_quantity(&p.name, &p.edm_type)
                && p.unit_path.is_none()
                && p.iso_currency_path.is_none()
        })
        .map(|p| p.name.as_str())
        .collect();
    if !decimal_without_unit.is_empty() {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Fields,
            "unit_missing",
            format!(
                "以下小数/金额类属性未关联 Measures.Unit 或 ISOCurrency：{}",
                decimal_without_unit.join(", "),
            ),
            "@Semantics.amount.currencyCode / @Semantics.quantity.unitOfMeasure",
            "使 Fiori 能使用正确的货币或计量单位格式化数值，否则只能显示原始小数。",
        ));
    }
    let code_without_text: Vec<&str> = et
        .properties
        .iter()
        .filter(|p| looks_code(&p.name) && p.text_path.is_none())
        .map(|p| p.name.as_str())
        .collect();
    if !code_without_text.is_empty() {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Fields,
            "text_missing",
            format!(
                "以下代码类属性未关联 Common.Text：{}",
                code_without_text.join(", "),
            ),
            "@ObjectModel.text.element",
            "将代码列与易读的描述列关联（Fiori 会根据 UI.TextArrangement 将它们组合显示）。",
        ));
    }
}

// ── Consistency ─────────────────────────────────────────────────────
// Contradictions inside the declared annotations. Warn-level because
// SAP services CAN run with these; the server returns 4xx the first
// time the user tries the bad combo — better to catch them up front.
//
// Stream order is contract: the seven rules below ship in the order
// `tests::consistency_rules_emit_in_canonical_order` asserts.
pub(super) fn check_consistency(et: &EntityType, out: &mut Vec<LintFinding>) {
    // 1. SelectionField points at a non-filterable property.
    {
        let bad: Vec<&str> = et
            .selection_fields
            .iter()
            .filter(|name| {
                et.properties
                    .iter()
                    .find(|p| &p.name == *name)
                    .is_some_and(|p| p.filterable == Some(false))
            })
            .map(String::as_str)
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Filtering,
                "selection_non_filterable",
                format!(
                    "UI.SelectionFields 引用了不可筛选的列：{}",
                    bad.join(", "),
                ),
                "@Consumption.filter.hidden + @UI.selectionField consistency",
                "Fiori 会在筛选栏中显示该筛选项，但服务器会拒绝对此字段使用 `$filter`；请统一配置。"
            ));
        }
    }
    // 2. SortOrder references a non-sortable property.
    {
        let bad: Vec<&str> = et
            .sort_order
            .iter()
            .filter_map(|s| {
                et.properties
                    .iter()
                    .find(|p| p.name == s.property)
                    .filter(|p| p.sortable == Some(false))
                    .map(|_| s.property.as_str())
            })
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::ListReport,
                "sort_non_sortable",
                format!(
                    "UI.PresentationVariant.SortOrder 引用了不可排序的列：{}",
                    bad.join(", "),
                ),
                "@Consumption.filter.sortable + SortOrder consistency",
                "服务器会对 `$orderby` 返回 400 错误，因此声明的默认排序无法生效。",
            ));
        }
    }
    // 3. UI.Hidden property appears in SelectionFields.
    {
        let bad: Vec<&str> = et
            .selection_fields
            .iter()
            .filter(|name| {
                et.properties
                    .iter()
                    .find(|p| &p.name == *name)
                    .is_some_and(|p| p.hidden)
            })
            .map(String::as_str)
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Filtering,
                "selection_hidden",
                format!(
                    "UI.SelectionFields 包含标记为 UI.Hidden 的列：{}",
                    bad.join(", "),
                ),
                "@UI.hidden + @UI.selectionField consistency",
                "Fiori 会隐藏该列，却仍将其作为筛选项提供，两项配置互相矛盾。",
            ));
        }
    }
    // 4. ValueList without Out/InOut parameter — no way to write
    //    back to the local property on pick.
    {
        let bad: Vec<&str> = et
            .properties
            .iter()
            .filter(|p| {
                p.value_list_variants.iter().any(|vl| {
                    !vl.parameters.iter().any(|param| {
                        matches!(
                            param.kind,
                            crate::metadata::ValueListParameterKind::InOut
                                | crate::metadata::ValueListParameterKind::Out
                        ) && param.local_property.is_some()
                    })
                })
            })
            .map(|p| p.name.as_str())
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Fields,
                "value_list_no_out",
                format!(
                    "Common.ValueList 未配置绑定到本地属性的 InOut/Out 参数：{}",
                    bad.join(", "),
                ),
                "@Consumption.valueHelpDefinition parameter mapping",
                "值帮助弹窗可以显示列表，但没有目标属性接收所选值——用户只能浏览，无法选择。",
            ));
        }
    }
    // 5. TextArrangement without a Common.Text to arrange.
    {
        let bad: Vec<&str> = et
            .properties
            .iter()
            .filter(|p| p.text_arrangement.is_some() && p.text_path.is_none())
            .map(|p| p.name.as_str())
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Fields,
                "text_arrangement_lonely",
                format!(
                    "以下属性声明了 UI.TextArrangement，但未关联 Common.Text：{}",
                    bad.join(", "),
                ),
                "@ObjectModel.text.element + @UI.textArrangement pair",
                "没有可供排列的描述文本；该显示方式提示缺少对应的描述列。",
            ));
        }
    }
    // 6. SelectionVariant references hidden or non-filterable property.
    //    The variant would surface a default filter that the column
    //    can't actually filter on — Fiori shows the chip, the server
    //    rejects the `$filter`. Walks both Parameters and SelectOptions
    //    since either form ends up in the generated `$filter`.
    {
        let mut bad: Vec<String> = Vec::new();
        for sv in &et.selection_variants {
            let names = sv
                .parameters
                .iter()
                .map(|p| p.property_name.as_str())
                .chain(sv.select_options.iter().map(|s| s.property_name.as_str()));
            for name in names {
                if let Some(p) = et.properties.iter().find(|p| p.name == name)
                    && (p.hidden || p.filterable == Some(false))
                    && !bad.iter().any(|b| b == name)
                {
                    bad.push(name.to_string());
                }
            }
        }
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Filtering,
                "selection_variant_dead_filter",
                format!(
                    "UI.SelectionVariant 引用了隐藏或不可筛选的列：{}",
                    bad.join(", "),
                ),
                "@UI.selectionVariant + filter/hidden consistency",
                "筛选变体为服务器不支持筛选的列生成了 `$filter` 子句，因此运行时无法应用该变体。",
            ));
        }
    }
    // 7. SemanticObject without SemanticKey. Fiori cross-app navigation
    //    forms the target URL from the SemanticObject + business-key
    //    parameters; without a SemanticKey it has no business key to
    //    pass, so the launchpad link will be malformed or generic.
    {
        let semantic_object_props: Vec<&str> = et
            .properties
            .iter()
            .filter(|p| p.semantic_object.is_some())
            .map(|p| p.name.as_str())
            .collect();
        if !semantic_object_props.is_empty() && et.semantic_keys.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Identity,
                "semantic_object_no_key",
                format!(
                    "属性 {} 声明了 Common.SemanticObject，但实体未声明 Common.SemanticKey。",
                    semantic_object_props.join(", "),
                ),
                "@Consumption.semanticObject + @ObjectModel.semanticKey",
                "跨应用导航不会携带业务键，导致启动板链接格式错误或打开通用首页。",
            ));
        }
    }
}

// ── Integrity (dangling-reference) checks ───────────────────────────
// These flag annotations whose Path/target references a property
// that doesn't exist on the entity. The usual cause is a column
// renamed in one CDS layer without the annotation being updated;
// the service still serves metadata but the target column won't
// resolve at runtime. Each finding names the source annotation and
// the bad target so the fix is obvious.
//
// Stream order is contract: the nine rules below ship in the order
// `tests::integrity_rules_emit_in_canonical_order` asserts.
pub(super) fn check_integrity(et: &EntityType, out: &mut Vec<LintFinding>) {
    let prop_names: std::collections::HashSet<&str> =
        et.properties.iter().map(|p| p.name.as_str()).collect();
    let nav_names: std::collections::HashSet<&str> =
        et.nav_properties.iter().map(|n| n.name.as_str()).collect();
    // A path resolves when it names a property of this entity, or starts
    // with one of its navigation (or complex-typed) properties
    // (`author/name`, `Address/City`). The rest of a multi-segment path
    // isn't checked — the linter sees one entity type at a time — so
    // only a bad first segment is reported. Annotation paths (`@…`) are
    // not property references.
    let resolves = |path: &str| -> bool {
        if prop_names.contains(path) || path.contains('@') {
            return true;
        }
        match path.split_once('/') {
            Some((first, rest)) => {
                !rest.is_empty() && (nav_names.contains(first) || prop_names.contains(first))
            }
            None => false,
        }
    };
    {
        let dangling: Vec<String> = et
            .properties
            .iter()
            .filter_map(|p| {
                p.text_path
                    .as_ref()
                    .filter(|target| !resolves(target.as_str()))
                    .map(|target| format!("{} → {}", p.name, target))
            })
            .collect();
        if !dangling.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "text_target_missing",
                format!(
                    "Common.Text 指向了此实体中不存在的列：{}",
                    dangling.join(", "),
                ),
                "@ObjectModel.text.element",
                "无法访问所引用的描述列，Fiori 将只显示原始代码而没有描述文本。",
            ));
        }
    }
    {
        let dangling: Vec<String> = et
            .properties
            .iter()
            .filter_map(|p| {
                p.unit_path
                    .as_ref()
                    .filter(|target| !resolves(target.as_str()))
                    .map(|target| format!("{} → {}", p.name, target))
            })
            .collect();
        if !dangling.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "unit_target_missing",
                format!(
                    "Measures.Unit / sap:unit 指向了不存在的列：{}",
                    dangling.join(", "),
                ),
                "@Semantics.quantity.unitOfMeasure",
                "数量列没有可解析的计量单位关联，Fiori 将显示原始数值。",
            ));
        }
    }
    {
        let dangling: Vec<String> = et
            .properties
            .iter()
            .filter_map(|p| {
                p.iso_currency_path
                    .as_ref()
                    .filter(|target| !resolves(target.as_str()))
                    .map(|target| format!("{} → {}", p.name, target))
            })
            .collect();
        if !dangling.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "currency_target_missing",
                format!(
                    "Measures.ISOCurrency 指向了不存在的列：{}",
                    dangling.join(", "),
                ),
                "@Semantics.amount.currencyCode",
                "金额列没有可解析的货币关联，Fiori 无法正确格式化该数值。",
            ));
        }
    }
    {
        let dangling: Vec<String> = et
            .properties
            .iter()
            .filter_map(|p| match &p.criticality {
                Some(crate::metadata::Criticality::Path(path)) if !resolves(path.as_str()) => {
                    Some(format!("{} → {}", p.name, path))
                }
                _ => None,
            })
            .collect();
        if !dangling.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "criticality_target_missing",
                format!(
                    "UI.Criticality Path 引用了不存在的列：{}",
                    dangling.join(", "),
                ),
                "@UI.criticality (Path form)",
                "单元格着色依赖的列无法解析，因此单元格不会显示颜色。",
            ));
        }
    }
    if let Some(hi) = &et.header_info
        && let Some(title) = &hi.title_path
        && !resolves(title.as_str())
    {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Integrity,
            "header_info_title_missing",
            format!("UI.HeaderInfo.Title 引用了不存在的列：{}", title,),
            "@UI.headerInfo.title.value",
            "对象页标题无法解析，Fiori 将回退为技术类型名称。",
        ));
    }
    {
        let bad: Vec<&str> = et
            .semantic_keys
            .iter()
            .filter(|n| !resolves(n.as_str()))
            .map(String::as_str)
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "semantic_key_target_missing",
                format!("Common.SemanticKey 引用了不存在的列：{}", bad.join(", "),),
                "@ObjectModel.semanticKey",
                "业务键提示指向了 Fiori 无法解析的列。",
            ));
        }
    }
    {
        let bad: Vec<&str> = et
            .selection_fields
            .iter()
            .filter(|n| !resolves(n.as_str()))
            .map(String::as_str)
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "selection_field_target_missing",
                format!("UI.SelectionFields 引用了不存在的列：{}", bad.join(", "),),
                "@UI.selectionField",
                "筛选栏中的筛选项将无法使用；用户输入值时服务器会返回 400 错误。",
            ));
        }
    }
    {
        let bad: Vec<String> = et
            .line_item
            .iter()
            .filter(|f| !resolves(f.value_path.as_str()))
            .map(|f| f.value_path.clone())
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "line_item_target_missing",
                format!("UI.LineItem DataField 引用了不存在的列：{}", bad.join(", "),),
                "@UI.lineItem",
                "列表报表列指向了实体中不存在的属性，运行时这些列会被静默忽略。",
            ));
        }
    }
    {
        let bad: Vec<&str> = et
            .sort_order
            .iter()
            .filter(|s| !resolves(s.property.as_str()))
            .map(|s| s.property.as_str())
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "sort_order_target_missing",
                format!(
                    "UI.PresentationVariant.SortOrder 引用了不存在的列：{}",
                    bad.join(", "),
                ),
                "@UI.presentationVariant.sortOrder",
                "声明的默认排序无法执行，Fiori 会静默忽略该排序子句。",
            ));
        }
    }
}

// ── Capabilities ────────────────────────────────────────────────────
// We don't reject services that omit these entirely (many transactional
// services do), but flag it as a hint.
pub(super) fn check_capabilities(et: &EntityType, out: &mut Vec<LintFinding>) {
    let any_capability = et.searchable.is_some()
        || et.countable.is_some()
        || et.top_supported.is_some()
        || et.skip_supported.is_some()
        || et.expandable.is_some()
        || et.properties.iter().any(|p| {
            p.filterable.is_some()
                || p.sortable.is_some()
                || p.creatable.is_some()
                || p.updatable.is_some()
                || p.required_in_filter.is_some()
        });
    if !any_capability {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Capabilities,
            "capabilities_silent",
            "未声明 Capabilities.* 注解——客户端无法预先验证支持范围，只能在运行时发现限制。"
                .to_string(),
            "@Search.searchable / @Search.defaultSearchElement / @Consumption.filter.*",
            "告知客户端服务实际支持的操作，使界面能提前禁用不支持的操作。",
        ));
    }
}

// ── Heuristic predicates ────────────────────────────────────────────
// Live alongside their sole consumers (check_identity uses
// is_uuid_like_key; check_fields uses the other two). Inline so a
// future contributor reading rules.rs sees the whole story without
// jumping files.

/// Heuristic: does the property name look UUID-ish? We only use this
/// to hint a missing SemanticKey, so false positives are cheap.
fn is_uuid_like_key(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with("uuid") || lower.ends_with("_id") && lower.contains("uuid")
}

/// Heuristic: does the property look like money / quantity, so it
/// *should* have a unit companion? We key off the edm_type (Decimal)
/// plus common name suffixes to cut false positives.
fn looks_monetary_or_quantity(name: &str, edm_type: &str) -> bool {
    if !edm_type.contains("Decimal") {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    lower.contains("amount")
        || lower.contains("price")
        || lower.contains("value")
        || lower.contains("quantity")
        || lower.contains("weight")
        || lower.contains("volume")
}

/// Heuristic: does the property look like a business code (`ID`,
/// `Code`) that a human-readable text column would pair with? Tuned
/// to avoid noisy hits on UUIDs, technical-numbering fields (`*Number`
/// often means an internal sequence, not a human-facing code), and
/// classifier columns that commonly stand alone.
fn looks_code(name: &str) -> bool {
    if name.len() < 3 {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    // Skip things that are ALREADY text columns by naming convention,
    // or UUID / internal-identifier fields that shouldn't pair with a
    // description at all.
    if lower.ends_with("name")
        || lower.ends_with("description")
        || lower.ends_with("text")
        || lower.ends_with("label")
        || lower.ends_with("uuid")
        || lower.ends_with("guid")
    {
        return false;
    }
    lower.ends_with("id") || lower.ends_with("code")
}
