// Excel export via rust_xlsxwriter. Produces a Summary sheet with the headline
// figures and a Transactions sheet listing every ledger entry for the semester.
// Money is written as decimal CNY numbers so the spreadsheet remains summable.
use rust_xlsxwriter::{Format, Workbook};

use crate::models::{Report, Transaction};

// Characters that spreadsheet software treats as the start of a formula.
const FORMULA_PREFIXES: [char; 10] = ['=', '+', '-', '@', '＝', '＋', '－', '＠', '\t', '\r'];

/// Neutralizes formula injection in user-controlled text. Prefixing such text
/// with a single quote forces it to be displayed as a literal value, so a
/// crafted source, purpose, or item cannot smuggle a formula into the workbook.
/// The full-width variants are included because localized spreadsheet software
/// may normalize them to the half-width formula markers.
fn sanitize_excel_text(value: &str) -> String {
    if value.starts_with(FORMULA_PREFIXES) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

/// Builds the whole workbook in memory and returns its bytes. The caller runs
/// this on the blocking pool because rust_xlsxwriter is synchronous and
/// CPU-bound.
pub fn build_excel(transactions: &[Transaction], report: &Report) -> anyhow::Result<Vec<u8>> {
    let mut workbook = Workbook::new();
    let header_format = Format::new()
        .set_bold()
        .set_background_color("#1f1f1f")
        .set_font_color("#ffffff");

    let summary_sheet = workbook.add_worksheet();
    summary_sheet.set_name("Summary")?;
    summary_sheet.set_column_width(0, 30)?;
    summary_sheet.set_column_width(1, 20)?;

    // The row cursor is tracked as u32 so no index conversion is needed.
    let mut row: u32 = 0;
    summary_sheet.write_string_with_format(row, 0, "Metric", &header_format)?;
    summary_sheet.write_string_with_format(row, 1, "Value", &header_format)?;
    row += 1;

    let money_rows = [
        ("Total income (CNY)", report.summary.total_income_cents),
        ("Total expense (CNY)", report.summary.total_expense_cents),
        ("Balance (CNY)", report.summary.balance_cents),
    ];
    for (label, cents) in &money_rows {
        summary_sheet.write_string(row, 0, *label)?;
        summary_sheet.write_number(row, 1, *cents as f64 / 100.0)?;
        row += 1;
    }

    summary_sheet.write_string(row, 0, "Income transactions")?;
    summary_sheet.write_number(row, 1, report.summary.income_count as f64)?;
    row += 1;
    summary_sheet.write_string(row, 0, "Expense transactions")?;
    summary_sheet.write_number(row, 1, report.summary.expense_count as f64)?;
    row += 2;

    summary_sheet.write_string_with_format(row, 0, "Top selling items", &header_format)?;
    summary_sheet.write_string_with_format(row, 1, "Quantity", &header_format)?;
    row += 1;
    for item in report.item_ranking.iter().take(10) {
        summary_sheet.write_string(row, 0, sanitize_excel_text(&item.item))?;
        summary_sheet.write_number(row, 1, item.quantity as f64)?;
        row += 1;
    }

    let transactions_sheet = workbook.add_worksheet();
    transactions_sheet.set_name("Transactions")?;
    let headers = [
        "ID",
        "Kind",
        "Amount (CNY)",
        "Source",
        "Purpose",
        "Item",
        "Operator",
        "Occurred at",
    ];
    for (column, header) in (0_u16..).zip(headers.iter()) {
        transactions_sheet.write_string_with_format(0, column, *header, &header_format)?;
    }
    transactions_sheet.set_column_width(3, 20)?;
    transactions_sheet.set_column_width(4, 24)?;
    transactions_sheet.set_column_width(7, 24)?;

    // Row 0 holds the header, so data rows start at 1.
    for (row, transaction) in (1_u32..).zip(transactions.iter()) {
        transactions_sheet.write_number(row, 0, transaction.id as f64)?;
        transactions_sheet.write_string(row, 1, sanitize_excel_text(&transaction.kind))?;
        transactions_sheet.write_number(row, 2, transaction.amount_cents as f64 / 100.0)?;
        transactions_sheet.write_string(
            row,
            3,
            sanitize_excel_text(transaction.source.as_deref().unwrap_or("")),
        )?;
        transactions_sheet.write_string(
            row,
            4,
            sanitize_excel_text(transaction.purpose.as_deref().unwrap_or("")),
        )?;
        transactions_sheet.write_string(
            row,
            5,
            sanitize_excel_text(transaction.item.as_deref().unwrap_or("")),
        )?;
        transactions_sheet.write_string(row, 6, sanitize_excel_text(&transaction.operator))?;
        transactions_sheet.write_string(row, 7, sanitize_excel_text(&transaction.occurred_at))?;
    }

    Ok(workbook.save_to_buffer()?)
}
