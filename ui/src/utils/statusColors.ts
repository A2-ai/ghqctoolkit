import type { QCStatus } from '~/api/issues'

/** Background color for swimlane headers / status badges, keyed by QC status. */
export const STATUS_LANE_COLOR: Record<QCStatus['status'], string> = {
  approved:               '#dcfce7',
  changes_after_approval: '#dcfce7',
  awaiting_review:        '#dbeafe',
  approval_required:      '#dbeafe',
  change_requested:       '#fee2e2',
  in_progress:            '#fef9c3',
  changes_to_comment:     '#fef9c3',
}
