# Finance Invariant Test Specification

## Waterfall and lineage

1. Baby expense allocates to Baby, then Electricity Baby, then Date.
2. Electricity Baby expense allocates to Electricity Baby, then Date.
3. Date expense is recorded directly without waterfall.
4. Personal, Travel, and Savings expenses remain direct.
5. Allocation legs sum exactly to the original expense.
6. Every new allocation leg has a valid, non-empty related_transaction_id.
7. Invalid or incomplete allocations roll back all writes.

## Reporting

8. Overview spending and monthly history count each original expense once.
9. Bucket spending uses allocation legs without double-counting.
10. finance_summary() and reporting functions do not mutate transaction history.

## Lifecycle and historical safety

11. Editing or deleting an original transaction updates or removes its allocation legs atomically.
12. Historical rows with missing lineage remain unchanged; no historical backfill occurs.
13. Appending new transactions preserves the existing historical CSV prefix.

## Test requirements

- Use isolated in-memory databases for database tests.
- Include boundary cases where an expense exactly fills a bucket and where it spills into the next bucket.
- Verify database state after both successful and failed writes.
- Do not run migration tests against the real finance database.
