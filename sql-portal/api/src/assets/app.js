/* Script portal progressive enhancement. Everything works without JavaScript;
 * these hooks only make searching, sorting and charts more convenient. The
 * page has no inline scripts (CSP), so behaviour is wired through data-* hooks. */
(function () {
  "use strict";

  var PALETTE = ["#2456d6", "#1d7a4b", "#d97706", "#b42318", "#7c3aed", "#0e7490", "#be185d", "#4d7c0f", "#475569"];

  function norm(text) {
    return (text || "").toLowerCase().replace(/\s+/g, " ").trim();
  }

  function plural(n, word) {
    return n + " " + word + (n === 1 ? "" : "s");
  }

  // ------------------------------------------------------ script list search
  function initScriptSearch() {
    var input = document.querySelector("[data-filter-scripts]");
    if (!input) return;
    var items = Array.prototype.slice.call(document.querySelectorAll("[data-filter-text]"));
    var groups = Array.prototype.slice.call(document.querySelectorAll("[data-group]"));
    var count = document.querySelector("[data-filter-count]");
    var empty = document.querySelector("[data-filter-empty]");

    function apply() {
      var query = norm(input.value);
      var shown = 0;
      items.forEach(function (li) {
        var match = !query || norm(li.getAttribute("data-filter-text")).indexOf(query) !== -1;
        li.hidden = !match;
        if (match) shown++;
      });
      // A section without matches is hidden, and its heading shows how many match.
      groups.forEach(function (group) {
        var visible = group.querySelectorAll("[data-filter-text]:not([hidden])").length;
        group.hidden = visible === 0;
        var badge = group.querySelector("[data-group-count]");
        if (badge) badge.textContent = visible;
      });
      if (count) count.textContent = plural(shown, "script") + " shown";
      if (empty) empty.hidden = shown !== 0;
    }

    input.addEventListener("input", apply);
    input.addEventListener("keydown", function (e) {
      if (e.key === "Escape") {
        input.value = "";
        apply();
      }
    });
    document.addEventListener("keydown", function (e) {
      var typing = /input|textarea|select/i.test(document.activeElement && document.activeElement.tagName);
      if (e.key === "/" && !typing && !e.ctrlKey && !e.metaKey) {
        e.preventDefault();
        input.focus();
        input.select();
      }
    });
  }

  // --------------------------------------------------------- run form niceties
  function initRunForm() {
    var form = document.querySelector("form[data-run-form]");
    if (!form) return;
    var results = document.getElementById("results");

    form.addEventListener("submit", function () {
      if (results) results.setAttribute("aria-busy", "true");
      var button = form.querySelector("button[type=submit]");
      if (button) button.disabled = true;
    });

    // Ctrl+Enter (Cmd+Enter on macOS) runs the script from any field.
    form.addEventListener("keydown", function (e) {
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && typeof form.requestSubmit === "function") {
        e.preventDefault();
        form.requestSubmit();
      }
    });

    var reset = form.querySelector("[data-reset]");
    if (reset) {
      reset.addEventListener("click", function () {
        form.reset(); // restores the defaults rendered into the HTML
        var first = form.querySelector("input, select");
        if (first) first.focus();
      });
    }
  }

  // ------------------------------------------------------------ result tables
  function cellText(td) {
    return td.textContent.replace(/\s+/g, " ").trim();
  }

  function isNumeric(td) {
    return td && td.classList.contains("num");
  }

  function sortTable(table, column, direction) {
    var tbody = table.tBodies[0];
    if (!tbody) return;
    var rows = Array.prototype.slice.call(tbody.rows);
    var numeric = rows.length > 0 && rows.every(function (r) {
      return isNumeric(r.cells[column]);
    });
    rows.sort(function (a, b) {
      var x = cellText(a.cells[column]);
      var y = cellText(b.cells[column]);
      var result;
      if (numeric) {
        result = parseFloat(x) - parseFloat(y);
      } else {
        result = x.localeCompare(y, undefined, { numeric: true, sensitivity: "base" });
      }
      return direction === "ascending" ? result : -result;
    });
    rows.forEach(function (r) { tbody.appendChild(r); });
  }

  function initSortableTable(table) {
    var headers = Array.prototype.slice.call(table.tHead ? table.tHead.rows[0].cells : []);
    var firstRow = table.tBodies[0] && table.tBodies[0].rows[0];
    headers.forEach(function (th, column) {
      if (firstRow && isNumeric(firstRow.cells[column])) th.classList.add("num-col");
      function toggle() {
        var next = th.getAttribute("aria-sort") === "ascending" ? "descending" : "ascending";
        headers.forEach(function (other) { other.setAttribute("aria-sort", "none"); });
        th.setAttribute("aria-sort", next);
        sortTable(table, column, next);
      }
      th.title = "Sort by " + th.textContent.trim();
      th.addEventListener("click", toggle);
      th.addEventListener("keydown", function (e) {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          toggle();
        }
      });
    });
  }

  function initTableFilter(table) {
    var tools = table.closest(".table-wrap").previousElementSibling;
    var input = tools && tools.querySelector("[data-table-filter]");
    if (!input) return;
    var count = tools.querySelector("[data-table-count]");
    var empty = table.closest(".table-wrap").querySelector("[data-table-empty]");
    var rows = Array.prototype.slice.call(table.tBodies[0].rows);
    var total = rows.length;

    function apply() {
      var query = norm(input.value);
      var shown = 0;
      rows.forEach(function (row) {
        var match = !query || norm(row.textContent).indexOf(query) !== -1;
        row.hidden = !match;
        if (match) shown++;
      });
      if (count) {
        count.textContent = query
          ? shown + " of " + plural(total, "row")
          : plural(total, "row");
      }
      if (empty) empty.hidden = shown !== 0;
    }
    input.addEventListener("input", apply);
    input.addEventListener("keydown", function (e) {
      if (e.key === "Escape") {
        input.value = "";
        apply();
      }
    });
  }

  function csvField(value) {
    var text = String(value);
    // Stop spreadsheets from evaluating cells that look like formulas.
    if (/^[=+\-@\t\r]/.test(text)) text = "'" + text;
    return /[",\n\r]/.test(text) ? '"' + text.replace(/"/g, '""') + '"' : text;
  }

  function exportCsv(table, filename) {
    var lines = [];
    var header = Array.prototype.slice.call(table.tHead.rows[0].cells).map(function (th) {
      return csvField(th.textContent.trim());
    });
    lines.push(header.join(","));
    Array.prototype.slice.call(table.tBodies[0].rows).forEach(function (row) {
      if (row.hidden) return;
      lines.push(Array.prototype.slice.call(row.cells).map(function (td) {
        return csvField(cellText(td));
      }).join(","));
    });
    var blob = new Blob([lines.join("\r\n") + "\r\n"], { type: "text/csv;charset=utf-8" });
    var url = URL.createObjectURL(blob);
    var link = document.createElement("a");
    link.href = url;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    link.remove();
    setTimeout(function () { URL.revokeObjectURL(url); }, 1000);
  }

  function initCsvExport(button, table) {
    button.addEventListener("click", function () {
      var script = window.location.pathname.split("/").filter(Boolean).pop() || "results";
      exportCsv(table, script + ".csv");
    });
  }

  function initTables() {
    Array.prototype.slice.call(document.querySelectorAll("table[data-sortable]")).forEach(function (table) {
      initSortableTable(table);
      initTableFilter(table);
      var tools = table.closest(".table-wrap").previousElementSibling;
      var button = tools && tools.querySelector("[data-export-csv]");
      if (button) initCsvExport(button, table);
    });
  }

  // ------------------------------------------------------------------ charts
  function chartFallback(canvas) {
    var card = canvas.closest(".chart-card");
    var note = card && card.querySelector("[data-chart-fallback]");
    if (note) note.hidden = false;
    canvas.hidden = true;
  }

  function initCharts() {
    var canvases = Array.prototype.slice.call(document.querySelectorAll("canvas[data-chart]"));
    if (canvases.length === 0) return;
    if (typeof window.Chart === "undefined") {
      canvases.forEach(chartFallback);
      return;
    }
    canvases.forEach(function (canvas) {
      var data = document.getElementById(canvas.getAttribute("data-chart"));
      try {
        var config = JSON.parse(data.textContent);
        var datasets = (config.data && config.data.datasets) || [];
        var labels = (config.data && config.data.labels) || [];
        var pie = config.type === "pie" || config.type === "doughnut";
        datasets.forEach(function (ds, i) {
          var colour = PALETTE[i % PALETTE.length];
          if (pie) {
            ds.backgroundColor = labels.map(function (_, j) { return PALETTE[j % PALETTE.length]; });
          } else {
            ds.backgroundColor = colour + "cc";
            ds.borderColor = colour;
            ds.borderWidth = config.type === "line" ? 2 : 1;
            ds.tension = config.type === "line" ? 0.25 : 0;
            ds.pointRadius = config.type === "scatter" ? 4 : 3;
          }
        });
        config.options = Object.assign(
          { responsive: true, maintainAspectRatio: false },
          config.options || {}
        );
        config.options.plugins = Object.assign(
          { legend: { position: pie ? "right" : "bottom", labels: { boxWidth: 12 } } },
          config.options.plugins || {}
        );
        new window.Chart(canvas, config);
      } catch (err) {
        chartFallback(canvas);
      }
    });
  }

  // ---------------------------------------------------------- share link
  // The link carries what the form holds now (even before a run), the current
  // page and the rows per page, so opening it shows the same view.
  function shareUrl() {
    var form = document.querySelector("form[data-run-form]");
    var params = form ? new URLSearchParams(new FormData(form)) : new URLSearchParams(window.location.search);
    params.set("_run", "1");
    var current = document.querySelector(".pager-bar [aria-current='page']");
    if (current) params.set("_page", current.textContent.trim());
    var size = document.querySelector("select[data-auto-submit]");
    if (size) params.set("_size", size.value);
    return window.location.origin + window.location.pathname + "?" + params.toString();
  }

  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(text);
    }
    return new Promise(function (resolve, reject) {
      var field = document.createElement("textarea");
      field.value = text;
      document.body.appendChild(field);
      field.select();
      var copied = document.execCommand("copy");
      field.remove();
      if (copied) resolve();
      else reject(new Error("copy failed"));
    });
  }

  function initShare() {
    var link = document.querySelector("[data-share-link]");
    if (!link) return;
    var output = document.querySelector("[data-share-output]");
    var field = document.querySelector("[data-share-url]");
    var note = document.querySelector("[data-share-note]");
    link.addEventListener("click", function (e) {
      e.preventDefault();
      var url = shareUrl();
      field.value = url;
      output.hidden = false;
      copyText(url).then(
        function () { note.textContent = "Link copied to the clipboard."; },
        function () { note.textContent = "Select the link and copy it."; }
      );
      field.select();
    });
  }

  // ------------------------------------------------------- rows per page
  // Changing the page size submits its form; the form returns to page 1.
  function initPageSize() {
    Array.prototype.slice.call(document.querySelectorAll("select[data-auto-submit]")).forEach(function (select) {
      select.addEventListener("change", function () {
        var form = select.form;
        if (!form) return;
        if (typeof form.requestSubmit === "function") form.requestSubmit();
        else form.submit();
      });
    });
  }

  // ------------------------------------------------------------------- boot
  function boot() {
    initScriptSearch();
    initRunForm();
    initTables();
    initPageSize();
    initShare();
    initCharts();
    var results = document.getElementById("results");
    if (results) results.removeAttribute("aria-busy");
  }

  // Back/forward navigation can restore a busy page from the cache.
  window.addEventListener("pageshow", function () {
    var results = document.getElementById("results");
    if (results) results.removeAttribute("aria-busy");
    var button = document.querySelector("form[data-run-form] button[type=submit]");
    if (button) button.disabled = false;
  });

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
})();
