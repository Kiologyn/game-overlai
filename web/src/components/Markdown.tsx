import { createContext, memo, useContext } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

/** How links open: in the overlay they must go to the default browser via the core. */
export const OpenLinkContext = createContext<((url: string) => void) | null>(null);

function Link({ href, children }: { href?: string; children?: React.ReactNode }) {
  const openLink = useContext(OpenLinkContext);
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer noopener"
      onClick={(e) => {
        if (openLink && href) {
          e.preventDefault();
          openLink(href);
        }
      }}
    >
      {children}
    </a>
  );
}

const components: Components = { a: Link };

/** react-markdown never renders raw HTML, so model output can't inject markup. */
export const Markdown = memo(function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
});
