// Excel export via rust_scriptwriter. Produces a Summary sheet with the headline
// figures and a Transactions sheet listing every ledger entry for the semester.
// Money is written as decimal CNY numbers so the spreadsheet remains summable.
use rust_xlsxwriter::{Format, Workbook};

use crate::models::{Report, Transaction};

pub fn build_excel(transactions: &[Transaction], report: &Report) -> anyhow::Result<Vec<u8>> {
    let mut workbook = Workbook::new();
    let header_format = Format::new()
        .set_bold()
        .set_background_color("#1f1f1f")
        .set_font_color("#ffffff");

    let summary = workbook.add_worksheet();
    summary.set_name("Summary")?;
    summary.set_column_width(0, 30)?;
    summary.set_column_width(1, 20)?;
    summary.write_string_with_format(0, 0, "Metric", &header_format)?;
    summary.write_string_with_format(0, 1, "Value", &header_format)?;

    let money_rows = [
        ("Total income (CNY)", report.summary.total_income_cents),
        ("Total expense (CNY)", report.summary.total_expense_cents),
        ("Balance (CNY)", report.summary.balance_cents),
    ];
    for (i, (label, cents)) in money_rows.iter().enumerate() {
        let row = (i + 1) as u32;
        summary.write_string(row, 0, *label)?;
        summary.write_number(row, 1, *cents as f64 / 100.0)?;
    }
    let mut row = (money_rows.len() + 1) as u32;
    summary.write_string(row, 0, "Income transactions")?;
    summary.write_number(row, 1, report.summary.income_count as f64)?;
    row += 1;
    summary.write_string(row, 0, "Expense transactions")?;
    summary.write_number(row, 1, report.summary.expense_count as f64)?;
    row += 2;
    summary.write_string_with_format(row, 0, "Top selling items", &header_format)?;
    summary.write_string_with_format(row, 1, "Quantity", &header_format)?;
    row += 1;
    for item in report.item_ranking.iter().take(10) {
        summary.write_string(row, 0, &item.item)?;
        summary.write_number(row, 1, item.quantity as f64)?;
        row += 1;
    }

    let data = workbook.add_worksheet();
    data.set_name("Transactions")?;
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
    for (col, header) in headers.iter().enumerate() {
        data.write_string_with_format(0, col as u16, *header, &header_format)?;
    }
    data.set_column_width(3, 20)?;
    data.set_column_width(4, 24)?;
    data.set_column_width(7, 24)?;
    for (i, tx) in transactions.iter().enumerate() {
        let r = (i + 1) as u32;
        data.write_number(r, 0, tx.id as f64)?;
        data.write_string(r, 1, &tx.kind)?;
        data.write_number(r, 2, tx.amount_cents as f64 / 100.0)?;
        data.write_string(r, 3, tx.source.as_deref().unwrap_or(""))?;
        data.write_string(r, 4, tx.purpose.as_deref().unwrap_or(""))?;
        data.write_string(r, 5, tx.item.as_deref().unwrap_or(""))?;
        data.write_string(r, 6, &tx.operator)?;
        data.write_string(r, 7, &tx.occurred_at)?;
    }

    Ok(workbook.save_to_buffer()?)
}
