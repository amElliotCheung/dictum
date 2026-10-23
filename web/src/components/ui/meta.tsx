import type { ReactNode } from 'react';
import { cn } from '../../lib/utils';

export function Meta({
  label,
  children,
  className,
}: {
  label: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div>
      <dt className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </dt>
      <dd className={cn('mt-0.5 text-sm', className)}>{children}</dd>
    </div>
  );
}
