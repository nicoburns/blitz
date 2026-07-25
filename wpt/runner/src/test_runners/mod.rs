use std::{fs, sync::Arc, time::Instant};

use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::{HtmlDocument, HtmlProvider};
use log::debug;

use crate::{SubtestCounts, TestFlags, TestKind, TestStatus, ThreadCtx};

mod attr_test;
mod harness_test;
mod ref_test;

pub use attr_test::process_attr_test;
pub use harness_test::process_harness_test;
pub use ref_test::process_ref_test;

/// Does the HTML contain an inline `<script>` (one without a `src` attribute)?
/// Used as a heuristic for "requires JavaScript to be executed".
pub fn has_inline_script(ctx: &ThreadCtx, html: &str) -> bool {
    ctx.inline_script_re
        .captures_iter(html)
        .any(|captures| !captures.get(1).unwrap().as_str().contains("src"))
}

pub struct SubtestResult {
    pub name: String,
    pub status: TestStatus,
    pub errors: Vec<String>,
}

pub fn process_test_file(
    ctx: &mut ThreadCtx,
    relative_path: &str,
) -> (
    TestKind,
    TestFlags,
    TestStatus,
    SubtestCounts,
    Vec<SubtestResult>,
) {
    debug!("Processing test file: {relative_path}");

    let file_contents = fs::read_to_string(ctx.wpt_dir.join(relative_path)).unwrap();

    // Compute flags
    let mut flags = TestFlags::empty();
    if ctx.float_re.is_match(&file_contents) {
        flags |= TestFlags::USES_FLOAT;
    }
    if ctx.intrinsic_re.is_match(&file_contents) {
        flags |= TestFlags::USES_INTRINSIC_SIZE;
    }
    if ctx.calc_re.is_match(&file_contents) {
        flags |= TestFlags::USES_CALC;
    }
    if ctx.direction_re.is_match(&file_contents) {
        flags |= TestFlags::USES_DIRECTION;
    }
    if ctx.writing_mode_re.is_match(&file_contents) {
        flags |= TestFlags::USES_WRITING_MODE;
    }
    if ctx.subgrid_re.is_match(&file_contents) {
        flags |= TestFlags::USES_SUBGRID;
    }
    if ctx.grid_lanes_re.is_match(&file_contents) {
        flags |= TestFlags::USES_GRID_LANES;
    }
    if ctx.script_re.is_match(&file_contents) {
        flags |= TestFlags::USES_SCRIPT;
    }

    // Ref Test
    let reference = ctx
        .reftest_re
        .captures(&file_contents)
        .and_then(|captures| captures.get(1).map(|href| href.as_str().to_string()));
    if let Some(reference) = reference {
        let counts = process_ref_test(
            ctx,
            relative_path,
            file_contents.as_str(),
            reference.as_str(),
            &mut flags,
        );

        let status = counts.as_status();
        return (TestKind::Ref, flags, status, counts, Vec::new());
    }

    // Attr Test
    let mut matches = ctx.attrtest_re.captures_iter(&file_contents);
    let first = matches.next();
    let second = matches.next();
    if first.is_some() && second.is_none() {
        // TODO: handle tests with multiple calls to checkLayout.
        #[allow(clippy::unnecessary_unwrap)]
        let captures = first.unwrap();
        let selector = captures.get(1).unwrap().as_str().to_string();
        drop(matches);

        debug!("{selector}");

        let (status, counts, results) =
            process_attr_test(ctx, &selector, &file_contents, relative_path);

        return (TestKind::Attr, flags, status, counts, results);
    }
    drop(matches);

    // Harness (testharness.js) test
    if ctx.harness_re.is_match(&file_contents) {
        let (status, counts, results) = process_harness_test(ctx, &file_contents, relative_path);
        return (TestKind::Harness, flags, status, counts, results);
    }

    // TODO: Handle other test formats.
    (
        TestKind::Unknown,
        flags,
        TestStatus::Skip,
        SubtestCounts::ZERO_OF_ZERO,
        Vec::new(),
    )
}

fn parse_and_resolve_document(
    ctx: &mut ThreadCtx,
    html: &str,
    relative_path: &str,
) -> BaseDocument {
    ctx.net_provider.reset();
    let config = DocumentConfig {
        base_url: Some(ctx.dummy_base_url.join(relative_path).unwrap().to_string()),
        font_ctx: Some(ctx.font_ctx.clone()),
        net_provider: Some(Arc::clone(&ctx.net_provider) as _),
        navigation_provider: Some(Arc::clone(&ctx.navigation_provider)),
        // Required for `innerHTML` support when the document is upgraded to
        // a `ScriptDocument` (harness tests and ref tests with scripts)
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        ..Default::default()
    };

    // `.xht`/`.xhtml` files must be parsed as XML: content sniffing cannot
    // detect all XHTML documents (e.g. ones with a plain `<!DOCTYPE html>`)
    let is_xml = relative_path.ends_with(".xht") || relative_path.ends_with(".xhtml");
    let mut document = if is_xml {
        HtmlDocument::from_xml(html, config)
    } else {
        HtmlDocument::from_html(html, config)
    };

    document.as_mut().set_viewport(ctx.viewport.clone());
    document.as_mut().resolve(0.0);
    pump_net_provider(ctx, document.as_mut());

    document.into()
}

/// Load pending resources (stylesheets, images, fonts), re-resolving the document
/// as they arrive. Loops because loading a resource may result in further
/// resources being requested.
pub fn pump_net_provider(ctx: &ThreadCtx, document: &mut BaseDocument) {
    let start = Instant::now();
    while ctx.net_provider.pending_item_count() > 0 {
        ctx.net_provider.for_each(|_| {});
        document.resolve(0.0);
        if Instant::now().duration_since(start).as_millis() > 500 {
            ctx.net_provider.log_pending_items();
            panic!(
                "Timeout. {} pending items.",
                ctx.net_provider.pending_item_count()
            );
        }
    }

    document.resolve(0.0);
}
