import { formatDate } from '../lib/format';
import type { Decision } from '../lib/types';
import { Markdown } from '../lib/markdown';
import { Card } from './ui/card';
import { Badge } from './ui/badge';

export function DecisionCard({ decision }: { decision: Decision }) {
  return (
    <Card className="p-3">
      <div className="mb-2 flex flex-wrap gap-1" aria-label="Topics">
        {decision.topics.map((topic) => <Badge key={topic.id} variant="secondary">{topic.name}</Badge>)}
      </div>
      <Markdown text={decision.statement} />
      <p className="mt-2 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        Rationale
      </p>
      <Markdown text={decision.rationale} />
      {decision.alternatives_rejected !== null ? (
        <>
          <h3 className="mt-2 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
            Alternatives rejected
          </h3>
          <Markdown text={decision.alternatives_rejected} />
        </>
      ) : null}
      <p className="mt-2 text-[11px] text-muted-foreground">
        Decision #{decision.id} · {formatDate(decision.created_at)}
      </p>
    </Card>
  );
}
