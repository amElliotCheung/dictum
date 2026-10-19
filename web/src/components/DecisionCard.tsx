import { formatDate } from '../lib/format';
import type { Decision } from '../lib/types';
import { Markdown } from '../lib/markdown';
import { Card } from './ui/card';

export function DecisionCard({ decision }: { decision: Decision }) {
  return (
    <Card className="p-3">
      <Markdown text={decision.statement} />
      <p className="mt-2 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        Rationale
      </p>
      <Markdown text={decision.rationale} />
      <p className="mt-2 text-[11px] text-muted-foreground">
        Decision #{decision.id} · {formatDate(decision.created_at)}
      </p>
    </Card>
  );
}
