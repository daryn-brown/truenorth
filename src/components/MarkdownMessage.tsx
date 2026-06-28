import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

// Tailwind v4 has no typography plugin here, so we map each markdown node to dark-theme styles by
// hand. This keeps the advisor's answers readable (headings, lists, bold figures, tables) without
// pulling `prose` in. Links open externally; code blocks scroll instead of overflowing the bubble.
const components: Components = {
  p: ({ children }) => <p className="my-2 leading-relaxed first:mt-0 last:mb-0">{children}</p>,
  h1: ({ children }) => (
    <h1 className="mb-2 mt-3 text-base font-semibold text-white first:mt-0">{children}</h1>
  ),
  h2: ({ children }) => (
    <h2 className="mb-2 mt-3 text-sm font-semibold text-white first:mt-0">{children}</h2>
  ),
  h3: ({ children }) => (
    <h3 className="mb-1 mt-3 text-sm font-semibold text-slate-100 first:mt-0">{children}</h3>
  ),
  ul: ({ children }) => <ul className="my-2 list-disc space-y-1 pl-5">{children}</ul>,
  ol: ({ children }) => <ol className="my-2 list-decimal space-y-1 pl-5">{children}</ol>,
  li: ({ children }) => <li className="leading-relaxed">{children}</li>,
  strong: ({ children }) => <strong className="font-semibold text-white">{children}</strong>,
  em: ({ children }) => <em className="italic">{children}</em>,
  a: ({ href, children }) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer noopener"
      className="text-indigo-300 underline underline-offset-2 hover:text-indigo-200"
    >
      {children}
    </a>
  ),
  blockquote: ({ children }) => (
    <blockquote className="my-2 border-l-2 border-slate-600 pl-3 text-slate-400">
      {children}
    </blockquote>
  ),
  hr: () => <hr className="my-3 border-slate-700" />,
  code: ({ className, children }) => {
    const isBlock = /language-/.test(className ?? "");
    if (isBlock) {
      return <code className={`${className ?? ""} text-xs`}>{children}</code>;
    }
    return (
      <code className="rounded bg-slate-900/60 px-1 py-0.5 font-mono text-[0.85em] text-slate-100">
        {children}
      </code>
    );
  },
  pre: ({ children }) => (
    <pre className="my-2 overflow-x-auto rounded-lg bg-slate-900/70 p-3 font-mono text-xs text-slate-100">
      {children}
    </pre>
  ),
  table: ({ children }) => (
    <div className="my-2 overflow-x-auto">
      <table className="w-full border-collapse text-xs">{children}</table>
    </div>
  ),
  thead: ({ children }) => <thead className="text-left text-slate-400">{children}</thead>,
  th: ({ children }) => (
    <th className="border-b border-slate-700 px-2 py-1 font-medium">{children}</th>
  ),
  td: ({ children }) => <td className="border-b border-slate-800 px-2 py-1">{children}</td>,
};

/** Render assistant markdown (GitHub-flavored) with the app's dark theme. */
function MarkdownMessage({ content }: { content: string }) {
  return (
    <div className="text-sm text-slate-200">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {content}
      </ReactMarkdown>
    </div>
  );
}

export default memo(MarkdownMessage);
