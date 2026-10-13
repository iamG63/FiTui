# Finance Invariants

## 1. Waterfall allocation

- Baby expenses allocate in order: Baby -> Electricity Baby -> Date.
- Electricity Baby expenses allocate in order: Electricity Baby -> Date.
- Date expenses are direct and must not waterfall.
- Other buckets, including Personal, Travel, and Savings, are direct.
- The transaction bucket, not its tag, determines allocation behavior.

## 2. Allocation correctness

- Allocation legs must be non-negative and sum to the original expense at supported currency precision.
- Persist the original transaction and all allocation legs atomically.
- Invalid or partial allocations must fail without writing incomplete transactions.

## 3. Lineage

- Every new allocation leg must have a non-empty related_transaction_id referencing its original transaction.
- Editing or deleting an original transaction must update or delete its allocation legs atomically.
- Never backfill lineage or modify historical transactions to manufacture lineage.

## 4. Reporting

- Overview totals, monthly history, and transaction counts must not double-count original expenses and allocation legs.
- Waterfall bucket spending uses allocation legs.
- Summary functions must not mutate transaction history.

## 5. Historical data safety

- Preserve all existing rows, including rows with missing lineage.
- Appending transactions must not change the existing CSV prefix.
- Do not create a fake transaction for opening balance.
- Do not silently rewrite stored bucket names during migration.

## 6. Budget rules

- Baby: 12.5%; Electricity Baby: 12.5%; Personal: 25%; Travel: 12.5%; Date: 12.5%; Savings: 25%.
- Baby and Electricity Baby remaining amounts cannot be negative; Date remaining may be negative.
- October 2026 already includes the 100,000 VND legacy adjustment for Baby, Electricity Baby, and Date. Do not apply it again.
- Future legacy adjustments stop when combined remaining for these three buckets is no longer negative.

## 7. Failure safety

- Reject invalid amounts and allocation plans before persistence.
- A failed allocation leaves no partial transaction or allocation legs.
- Migrations must preserve historical transaction rows.
