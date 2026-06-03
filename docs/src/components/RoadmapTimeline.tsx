import React, {type ReactNode, useEffect, useRef, useState} from 'react';

type ItemType = 'version' | 'done' | 'active' | 'future';

type RoadmapItemProps = {
  type: ItemType;
  title: string;
  desc?: string;
  label?: string;
  version?: string;
  date?: string;
  num?: number;
  children?: ReactNode;
};

export function RoadmapItem({
  type,
  title,
  desc,
  label,
  version,
  date,
  num,
  children,
}: RoadmapItemProps) {
  const [open, setOpen] = useState(false);
  const [maxH, setMaxH] = useState<string>('0px');
  const bodyRef = useRef<HTMLDivElement>(null);
  const hasBody = Boolean(children);

  useEffect(() => {
    const node = bodyRef.current;
    if (!node) return;
    if (open) {
      const h = `${node.scrollHeight}px`;
      setMaxH(h);
      const t = window.setTimeout(() => setMaxH('none'), 380);
      return () => window.clearTimeout(t);
    }
    if (maxH === 'none') {
      const h = `${node.scrollHeight}px`;
      setMaxH(h);
      requestAnimationFrame(() => setMaxH('0px'));
      return;
    }
    setMaxH('0px');
  }, [open]);

  const stemLabel = label ?? (type === 'version' ? date : '') ?? '';

  const nodeEl = (() => {
    if (type === 'done')
      return (
        <svg
          viewBox="0 0 16 16"
          width="11"
          height="11"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.4"
          strokeLinecap="round"
          strokeLinejoin="round">
          <polyline points="3,8 6.5,12 13,4" />
        </svg>
      );
    if (type === 'active')
      return (
        <svg viewBox="0 0 16 16" width="8" height="8" fill="currentColor">
          <circle cx="8" cy="8" r="4" />
        </svg>
      );
    if (type === 'version') return null;
    return <span className="roadmap-item__num">{num}</span>;
  })();

  return (
    <div
      className={`roadmap-item roadmap-item--${type}${open ? ' roadmap-item--open' : ''}`}>
      <div className="roadmap-item__label">{stemLabel}</div>

      <div className="roadmap-item__stem">
        <div className="roadmap-item__node">{nodeEl}</div>
      </div>

      <div className="roadmap-item__card">
        <button
          type="button"
          className="roadmap-item__trigger"
          onClick={() => hasBody && setOpen(v => !v)}
          aria-expanded={hasBody ? open : undefined}
          style={{cursor: hasBody ? 'pointer' : 'default'}}>
          <div className="roadmap-item__header">
            <span className="roadmap-item__title">{title}</span>
            <span className="roadmap-item__spacer" />
            {version && (
              <span className="roadmap-version-tag">{version}</span>
            )}
            {hasBody && (
              <span className="roadmap-item__chevron" aria-hidden="true" />
            )}
          </div>
          {desc && <p className="roadmap-item__desc">{desc}</p>}
        </button>

        {hasBody && (
          <div
            className="roadmap-item__body"
            style={{
              maxHeight: maxH,
              overflow: 'hidden',
              transition: 'max-height 380ms cubic-bezier(0.22,1,0.36,1)',
            }}>
            <div ref={bodyRef} className="roadmap-item__body-inner">
              {children}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export default function RoadmapTimeline({children}: {children: ReactNode}) {
  return <div className="roadmap-timeline">{children}</div>;
}
