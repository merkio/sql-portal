//! Every way the page presents selected data, checked against the showcase
//! scripts in the demo configuration (data from migrations 0003 and 0004).

use sql_portal_integration_tests::harness as common;

use axum::http::StatusCode;
use common::{harness, Options};

async fn page(uri: &str) -> (StatusCode, String) {
    let h = harness(Options {
        auth_enabled: false,
        ..Options::default()
    })
    .await;
    let r = h.get(uri, None).await;
    (r.status, r.body)
}

#[tokio::test]
async fn table_only_is_the_default_presentation() {
    let (status, body) = page("/scripts/showcase_table?_run=1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<table"));
    assert!(body.contains("24 rows in"));
    assert!(!body.contains("data-chart="), "no charts were configured");
}

#[tokio::test]
async fn bar_chart_draws_two_series_with_the_table() {
    let (_, body) = page("/scripts/showcase_bar?_run=1").await;
    assert!(body.contains("\"type\":\"bar\""));
    assert!(body.contains("\"label\":\"orders\"") && body.contains("\"label\":\"total\""));
    assert!(body.contains("<table"));
}

#[tokio::test]
async fn line_chart_keeps_gaps_for_missing_values() {
    let (_, body) = page("/scripts/showcase_line?_run=1").await;
    assert!(body.contains("\"type\":\"line\""));
    // May and June 2023 have no orders: the series contains nulls, not zeros.
    assert!(body.contains("null"), "missing values must stay gaps");
}

#[tokio::test]
async fn pie_and_doughnut_draw_their_kind() {
    let (_, pie) = page("/scripts/showcase_pie?_run=1").await;
    assert!(pie.contains("\"type\":\"pie\"") && pie.contains("<table"));

    let (_, doughnut) = page("/scripts/showcase_doughnut?_run=1").await;
    assert!(doughnut.contains("\"type\":\"doughnut\""));
    assert!(!doughnut.contains("<table"), "table: false hides the table");
}

#[tokio::test]
async fn scatter_plots_points_without_a_table() {
    let (_, body) = page("/scripts/showcase_scatter?_run=1").await;
    assert!(body.contains("\"type\":\"scatter\""));
    assert!(body.contains("\"x\":"), "scatter points carry x and y");
    assert!(!body.contains("<table"));
}

#[tokio::test]
async fn several_charts_and_a_table_on_one_page() {
    let (_, body) = page("/scripts/showcase_mixed?_run=1").await;
    for i in 0..3 {
        assert!(
            body.contains(&format!("data-chart=\"chart-data-{i}\"")),
            "chart {i}"
        );
    }
    assert!(
        body.contains("\"type\":\"bar\"")
            && body.contains("\"type\":\"pie\"")
            && body.contains("\"type\":\"doughnut\"")
    );
    assert!(body.contains("<table"));
}

#[tokio::test]
async fn paged_results_with_capped_charts_say_both() {
    let (_, body) = page("/scripts/showcase_truncated?_run=1").await;
    assert!(body.contains("50 rows in"), "one page of 50 is shown");
    assert!(body.contains("rows 1–50 of 5000"));
    assert!(body.contains("charts use the first 200 rows"));
    assert!(body.contains("data-chart=\"chart-data-0\""));
}

#[tokio::test]
async fn empty_results_show_empty_states() {
    let (status, body) = page("/scripts/showcase_empty?_run=1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("returned no rows"));
    assert!(!body.contains("<table"));
}

#[tokio::test]
async fn a_broken_chart_does_not_hide_the_others() {
    let (status, body) = page("/scripts/showcase_chart_error?_run=1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Chart unavailable: column &#x27;missing_column&#x27; not found"));
    assert!(
        body.contains("data-chart=\"chart-data-1\""),
        "the valid chart still draws"
    );
}

#[tokio::test]
async fn every_parameter_kind_is_rendered_as_an_input() {
    let (_, body) = page("/scripts/showcase_params?_run=1").await;
    assert!(body.contains("type=\"text\""));
    assert!(body.contains("type=\"number\" step=\"1\""), "integer input");
    assert!(
        body.contains("type=\"number\" step=\"any\""),
        "number input"
    );
    assert!(body.contains("type=\"checkbox\""), "boolean input");
    assert!(body.contains("type=\"date\""));
    assert!(body.contains("<select"));
    assert!(body.contains("Matches label or city."), "help text");
}

#[tokio::test]
async fn invalid_parameter_values_are_reported_next_to_their_field() {
    let (status, body) = page(
        "/scripts/showcase_params?_run=1&status=bogus&from_date=2024-01-01&to_date=2024-12-31",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("not an allowed option"));
    assert!(body.contains("field invalid"));
}

#[tokio::test]
async fn column_types_render_nulls_booleans_text_and_escaped_markup() {
    let (_, body) = page("/scripts/showcase_column_types?_run=1").await;
    assert!(
        body.contains("Kraków") && body.contains("東京"),
        "non-ASCII text"
    );
    assert!(
        body.contains("&lt;script&gt;alert(1)&lt;&#x2f;script&gt;"),
        "markup is escaped"
    );
    assert!(!body.contains("<script>alert(1)"));
    assert!(
        body.contains("<td class=\"num\">1234.5678</td>"),
        "decimals keep their digits"
    );
}

#[tokio::test]
async fn wide_tables_keep_every_column() {
    let (_, body) = page("/scripts/showcase_wide_table?_run=1").await;
    let headers = body.matches("<th scope=\"col\"").count();
    assert_eq!(headers, 12, "all twelve columns are in the header");
    assert!(body.contains("25 rows in"), "one page of 25 is shown");
    assert!(body.contains("rows 1–25 of 30"));
}

#[tokio::test]
async fn pages_are_served_by_number() {
    let (_, second) = page("/scripts/showcase_big_table?_run=1&_page=2").await;
    assert!(second.contains("rows 101–200 of 20000"));
    assert!(second.contains("<td class=\"num\">101</td>"));
    assert!(
        !second.contains("item-00100<"),
        "the previous page is not repeated"
    );
}

#[tokio::test]
async fn out_of_range_or_invalid_pages_are_clamped() {
    let (_, last) = page("/scripts/showcase_big_table?_run=1&_page=999").await;
    assert!(last.contains("rows 19901–20000 of 20000"));
    let (_, zero) = page("/scripts/showcase_big_table?_run=1&_page=0").await;
    assert!(zero.contains("rows 1–100 of 20000"));
    let (_, junk) = page("/scripts/showcase_big_table?_run=1&_page=abc").await;
    assert!(junk.contains("rows 1–100 of 20000"));
}

#[tokio::test]
async fn pager_links_keep_the_form_values() {
    let (_, body) = page("/scripts/showcase_search_paged?_run=1&q=gadget&_page=2").await;
    assert!(body.contains("rows 11–20 of 45"));
    assert!(
        body.contains("href=\"?q=gadget&amp;_run=1&amp;_page=3\""),
        "next keeps the search"
    );
    assert!(body.contains("aria-current=\"page\""));
}

#[tokio::test]
async fn the_pager_has_every_page_for_a_big_result() {
    let (_, body) = page("/scripts/showcase_big_table?_run=1").await;
    assert!(
        body.contains("aria-label=\"Page 200\""),
        "the last page is linked"
    );
    assert!(
        body.contains("…"),
        "distant pages are summarised with a gap"
    );
    assert!(body.contains("rel=\"next\""));
    assert!(
        !body.contains("rel=\"prev\""),
        "the first page has no previous link"
    );
}

#[tokio::test]
async fn scripts_without_a_table_do_not_page() {
    let (_, body) = page("/scripts/showcase_scatter?_run=1&_page=2").await;
    assert!(!body.contains("class=\"pager\""));
    assert!(body.contains("\"type\":\"scatter\""));
}

#[tokio::test]
async fn filter_scripts_draw_their_charts_for_the_chosen_filter() {
    let (_, bar_pie) = page("/scripts/showcase_filter_bar_pie?_run=1&region=EU&from_date=2024-01-01&to_date=2024-06-30&min_amount=").await;
    assert!(bar_pie.contains("\"type\":\"bar\"") && bar_pie.contains("\"type\":\"pie\""));

    let (_, monthly) = page("/scripts/showcase_filter_monthly_line?_run=1&region=US&from_date=2024-01-01&to_date=2024-12-31").await;
    assert!(monthly.contains("\"type\":\"line\""));

    let (_, top) = page("/scripts/showcase_top_n_switch?_run=1&limit=2&include_apac=0").await;
    assert!(top.contains("2 rows in"), "the limit is applied");
    assert!(top.contains("\"type\":\"doughnut\""));

    let (_, scatter) =
        page("/scripts/showcase_scatter_filters?_run=1&segment=consumer&min_advertising=").await;
    assert!(
        scatter.contains("rows 1–10 of 20"),
        "one segment of sixty points"
    );
    assert!(scatter.contains("\"type\":\"scatter\""));
}

#[tokio::test]
async fn search_without_matches_is_an_empty_paged_result() {
    let (status, body) = page("/scripts/showcase_search_paged?_run=1&q=no-such-thing").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("returned no rows"));
    assert!(!body.contains("class=\"pager\""), "nothing to page through");
}

#[tokio::test]
async fn one_page_shows_no_pager_at_all() {
    let (_, body) = page("/scripts/showcase_table?_run=1").await;
    assert!(body.contains("24 rows in"));
    assert!(
        !body.contains("class=\"pager\""),
        "a single page needs no buttons"
    );
    assert!(!body.contains("pager-arrow"));
}

#[tokio::test]
async fn pager_shows_arrows_around_the_active_page_and_the_total() {
    let (_, body) = page("/scripts/showcase_big_table?_run=1&_page=7").await;
    assert!(body.contains("Page 7 of 200"));
    // Arrows sit on both sides of the active page's neighbourhood.
    let bar = body
        .split("class=\"pager-bar\"")
        .nth(1)
        .unwrap()
        .split("</ol>")
        .next()
        .unwrap();
    assert!(bar.find("rel=\"prev\"").unwrap() < bar.find("aria-current").unwrap());
    assert!(bar.find("aria-current").unwrap() < bar.find("rel=\"next\"").unwrap());
    assert!(bar.contains("pager-gap"), "gaps summarise distant pages");
}

#[tokio::test]
async fn first_and_last_pages_disable_the_missing_arrow() {
    let (_, first) = page("/scripts/showcase_big_table?_run=1").await;
    assert!(!first.contains("rel=\"prev\""));
    assert!(first.contains("No previous page"));
    let (_, last) = page("/scripts/showcase_big_table?_run=1&_page=200").await;
    assert!(!last.contains("rel=\"next\""));
    assert!(last.contains("No next page"));
}

#[tokio::test]
async fn gallery_scripts_each_draw_several_chart_kinds() {
    let cases = [
        (
            "gallery_all_types",
            5,
            &["line", "bar", "pie", "doughnut", "scatter"][..],
        ),
        ("gallery_quarterly_bars", 2, &["bar", "line"][..]),
        ("gallery_brand_share", 2, &["pie", "doughnut"][..]),
        ("gallery_sensor_readings", 2, &["line", "scatter"][..]),
        ("gallery_survey_bands", 3, &["bar", "line", "doughnut"][..]),
        ("gallery_web_traffic", 2, &["line", "bar"][..]),
    ];
    for (name, charts, kinds) in cases {
        let (status, body) = page(&format!("/scripts/{name}?_run=1")).await;
        assert_eq!(status, StatusCode::OK, "{name}");
        assert_eq!(
            body.matches("data-chart=\"chart-data-").count(),
            charts,
            "{name}"
        );
        for kind in kinds {
            assert!(
                body.contains(&format!("\"type\":\"{kind}\"")),
                "{name} draws {kind}"
            );
        }
        assert!(!body.contains("Chart unavailable"), "{name}");
    }
}

#[tokio::test]
async fn gallery_filters_change_the_data() {
    let (_, all) = page("/scripts/gallery_quarterly_bars?_run=1&year=ALL").await;
    assert!(all.contains("rows 1–8 of 8") || all.contains("8 rows in"));
    let (_, one_year) = page("/scripts/gallery_quarterly_bars?_run=1&year=2024").await;
    assert!(one_year.contains("4 rows in"), "one year has four quarters");

    let (_, brands) = page("/scripts/gallery_brand_share?_run=1&q=ini&min_share=").await;
    assert!(
        brands.contains("1 row in"),
        "text search narrows the brands"
    );

    let (_, weekdays) = page("/scripts/gallery_web_traffic?_run=1&weekdays_only=1").await;
    assert!(weekdays.contains("of 43"), "weekends are switched off");

    let (_, sensors) = page("/scripts/gallery_sensor_readings?_run=1&location=Office&from_date=2024-06-01&to_date=2024-06-02").await;
    assert!(sensors.contains("of 48"), "two days of hourly readings");
}

#[tokio::test]
async fn the_script_list_is_grouped_in_order() {
    let (status, body) = page("/").await;
    assert_eq!(status, StatusCode::OK);
    let headings: Vec<&str> = body
        .split("<h2 class=\"group-title\">")
        .skip(1)
        .map(|part| part.split(' ').next().unwrap())
        .collect();
    assert_eq!(headings, vec!["Charts", "Sales", "Showcase"], "{body}");
    assert_eq!(body.matches("data-group-count").count(), 3);
}

#[tokio::test]
async fn api_lists_each_scripts_group() {
    let h = harness(Options {
        auth_enabled: false,
        ..Options::default()
    })
    .await;
    let r = h.get("/api/scripts", None).await;
    let scripts = r.json()["scripts"].as_array().unwrap().clone();
    let find = scripts.iter().find(|s| s["name"] == "find_sales").unwrap();
    assert_eq!(find["group"], "Sales");
}

#[tokio::test]
async fn rows_per_page_offers_the_common_sizes_and_selects_the_current_one() {
    let (_, body) = page("/scripts/showcase_big_table?_run=1").await;
    assert!(
        body.contains("name=\"_size\""),
        "the table is paged, so the choice is shown"
    );
    for size in [5, 10, 25, 50, 100] {
        assert!(
            body.contains(&format!("<option value=\"{size}\"")),
            "option {size}"
        );
    }
    // The script's own page size (100) is the selected choice.
    assert!(body.contains("<option value=\"100\" selected>"));
    assert!(
        body.contains("data-auto-submit"),
        "changing it submits the form"
    );
}

#[tokio::test]
async fn a_chosen_size_changes_the_page_length_and_resets_the_page() {
    let (_, body) = page("/scripts/showcase_big_table?_run=1&_size=10&_page=3").await;
    assert!(body.contains("rows 21–30 of 20000"));
    assert!(body.contains("Page 3 of 2000"));
    assert!(body.contains("<option value=\"10\" selected>"));
    assert!(
        body.contains("name=\"_page\" value=\"1\""),
        "a size change starts at page 1"
    );
}

#[tokio::test]
async fn the_chosen_size_is_kept_by_the_page_links() {
    let (_, body) = page("/scripts/showcase_big_table?_run=1&_size=10&_page=2").await;
    assert!(
        body.contains("href=\"?_run=1&amp;_page=3&amp;_size=10\""),
        "{body}"
    );
}

#[tokio::test]
async fn an_invalid_size_falls_back_to_the_script_default() {
    let (_, body) = page("/scripts/showcase_big_table?_run=1&_size=abc").await;
    assert!(body.contains("rows 1–100 of 20000"));
    let (_, zero) = page("/scripts/showcase_big_table?_run=1&_size=0").await;
    assert!(zero.contains("rows 1–100 of 20000"));
}

#[tokio::test]
async fn the_size_choice_is_removed_where_nothing_is_paged() {
    let (_, scatter) = page("/scripts/showcase_scatter?_run=1").await;
    assert!(
        !scatter.contains("name=\"_size\""),
        "a script without a table has no choice"
    );
    assert!(!scatter.contains("data-auto-submit"));
}

#[tokio::test]
async fn share_link_carries_the_parameters_of_the_view() {
    let (_, body) =
        page("/scripts/sales_by_region?_run=1&from_date=2024-01-01&to_date=2024-03-31").await;
    assert!(body.contains("data-share-link"));
    let href = body
        .split("data-share-link")
        .next()
        .unwrap()
        .rsplit("href=\"")
        .next()
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert!(href.contains("from_date=2024-01-01"), "{href}");
    assert!(href.contains("to_date=2024-03-31"), "{href}");
    assert!(
        href.contains("_run=1") && href.contains("_page=1") && href.contains("_size=25"),
        "{href}"
    );
}

#[tokio::test]
async fn share_link_keeps_the_page_and_size_and_escapes_values() {
    let (_, body) = page("/scripts/showcase_big_table?_run=1&_size=10&_page=3").await;
    assert!(
        body.contains("href=\"?_page=3&amp;_size=10&amp;_run=1\"")
            || body.contains("_page=3&amp;_size=10"),
        "{body}"
    );

    let (_, search) = page("/scripts/showcase_search_paged?_run=1&q=a%26b%3Dc").await;
    assert!(
        search.contains("q=a%26b%3Dc"),
        "a value with & and = stays one value"
    );
}

#[tokio::test]
async fn share_link_of_a_page_that_runs_on_open_includes_the_run() {
    // A script whose required parameters all have defaults runs when opened.
    let (_, body) = page("/scripts/showcase_params").await;
    assert!(body.contains("href=\"?_run=1"), "{body}");
    assert!(
        body.contains("data-share-output"),
        "the result of the share is prepared"
    );
}

#[tokio::test]
async fn locked_scripts_are_hidden_by_default_and_shown_on_request() {
    // Signed in as alice, who may not run the admin-only "Order sizes" script.
    let h = harness(Options::default()).await;
    let cookie = h.login_ldap("alice", "secret").await;

    let default = h.get("/", Some(&cookie)).await.body;
    assert!(
        !default.contains("Order sizes"),
        "locked scripts are not listed by default"
    );
    assert!(
        default.contains("href=\"/?all=1\""),
        "the toggle offers the locked ones"
    );

    let all = h.get("/?all=1", Some(&cookie)).await.body;
    assert!(
        all.contains("Order sizes (admin)"),
        "the locked script is listed"
    );
    assert!(all.contains("class=\"script-card locked\""));
    assert!(all.contains("Requires all of"));
    assert!(
        all.contains("<code>scripts.admin.read</code>"),
        "required permissions are shown"
    );
    assert!(
        all.contains("Quantity versus amount for one region"),
        "the description of a locked script is shown"
    );
    assert!(
        !all.contains("href=\"/scripts/order_sizes\""),
        "a locked script cannot be opened"
    );
    assert!(
        all.contains("href=\"/scripts/find_sales\""),
        "accessible scripts still open"
    );
    assert!(all.contains("Hide locked scripts"));
}

#[tokio::test]
async fn api_marks_locked_scripts_and_lists_them_on_request() {
    let h = harness(Options::default()).await;
    let cookie = h.login_ldap("alice", "secret").await;

    let default = h.get("/api/scripts", Some(&cookie)).await.json();
    let names: Vec<&str> = default["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"order_sizes"));

    let all = h.get("/api/scripts?all=1", Some(&cookie)).await.json();
    let locked = all["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "order_sizes")
        .expect("listed with all=1");
    assert_eq!(locked["accessible"], false);
    assert_eq!(
        locked["required_permissions"],
        serde_json::json!(["scripts.admin.read"])
    );
    assert!(locked["description"]
        .as_str()
        .unwrap()
        .contains("Quantity versus amount"));
}
