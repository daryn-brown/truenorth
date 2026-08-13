import type { ReactNode } from "react";
import BrandMark from "../../shared/BrandMark";
import "./landing.css";

const REPOSITORY_URL = "https://github.com/daryn-brown/truenorth";
const RELEASES_URL = `${REPOSITORY_URL}/releases/latest`;

const ArrowIcon = () => (
  <svg viewBox="0 0 20 20" aria-hidden="true">
    <path d="M4 10H16M11 5L16 10L11 15" />
  </svg>
);

const LockIcon = () => (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="5" y="10" width="14" height="11" rx="4" />
    <path d="M8.5 10V7.5a3.5 3.5 0 0 1 7 0V10M12 14.5V17" />
  </svg>
);

const GlobeIcon = () => (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <circle cx="12" cy="12" r="9" />
    <path d="M3.5 12H20.5M12 3c2.3 2.5 3.5 5.5 3.5 9S14.3 18.5 12 21M12 3C9.7 5.5 8.5 8.5 8.5 12s1.2 6.5 3.5 9" />
  </svg>
);

const ChartIcon = () => (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M4 19V10M10 19V5M16 19v-7M22 19V2" />
  </svg>
);

const SparkIcon = () => (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="m12 3 1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6L12 3ZM19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8L19 15Z" />
  </svg>
);

const features: Array<{
  number: string;
  title: string;
  description: string;
  icon: ReactNode;
}> = [
  {
    number: "01",
    title: "One net worth, every country",
    description:
      "See bank, brokerage, retirement, property, and manual accounts together in the currency that feels like home.",
    icon: <GlobeIcon />,
  },
  {
    number: "02",
    title: "See what exchange rates changed",
    description:
      "Separate real portfolio progress from currency movement with clear historical FX context.",
    icon: <ChartIcon />,
  },
  {
    number: "03",
    title: "Private by design",
    description:
      "Your financial history stays encrypted on your device. Connections are read-only and always under your control.",
    icon: <LockIcon />,
  },
  {
    number: "04",
    title: "Advice that sees the whole picture",
    description:
      "Ask questions across accounts and borders without copying sensitive screenshots into a generic chatbot.",
    icon: <SparkIcon />,
  },
];

const currencies = [
  { code: "CAD", share: "48%", color: "violet" },
  { code: "USD", share: "39%", color: "blue" },
  { code: "JMD", share: "13%", color: "mint" },
];

function Brand() {
  return (
    <span className="landing-brand">
      <BrandMark className="landing-brand__mark" />
      <span>
        <strong>TrueNorth</strong>
        <small>cross-border wealth</small>
      </span>
    </span>
  );
}

function WealthPreview() {
  return (
    <div
      className="wealth-preview"
      role="img"
      aria-label="Illustrative TrueNorth dashboard showing a combined cross-border net worth"
    >
      <div className="wealth-preview__aurora wealth-preview__aurora--top" />
      <div className="wealth-preview__aurora wealth-preview__aurora--bottom" />
      <div className="wealth-preview__orbit wealth-preview__orbit--one" />
      <div className="wealth-preview__orbit wealth-preview__orbit--two" />

      <article className="preview-card currency-card">
        <div className="preview-card__heading">
          <span>Currency exposure</span>
          <span className="status-dot">Live</span>
        </div>
        <div className="currency-chart">
          <div className="currency-chart__ring">
            <span>3</span>
            <small>currencies</small>
          </div>
          <div className="currency-chart__legend">
            {currencies.map((currency) => (
              <div key={currency.code}>
                <i className={`currency-dot currency-dot--${currency.color}`} />
                <span>{currency.code}</span>
                <strong>{currency.share}</strong>
              </div>
            ))}
          </div>
        </div>
      </article>

      <article className="preview-card world-card">
        <div className="preview-card__heading">
          <span>Your financial world</span>
          <GlobeIcon />
        </div>
        <div className="world-map" aria-hidden="true">
          <span className="world-map__line world-map__line--one" />
          <span className="world-map__line world-map__line--two" />
          <span className="country-node country-node--ca">
            <i>CA</i>
            <small>Home</small>
          </span>
          <span className="country-node country-node--us">
            <i>US</i>
            <small>Work</small>
          </span>
          <span className="country-node country-node--jm">
            <i>JM</i>
            <small>Family</small>
          </span>
        </div>
      </article>

      <article className="balance-card">
        <div className="balance-card__topline">
          <span>Combined net worth</span>
          <span className="private-pill">
            <LockIcon />
            Private
          </span>
        </div>
        <div className="balance-card__amount">
          <span>CA$</span>
          742,580
        </div>
        <div className="balance-card__change">
          <strong>↗ 2.8%</strong>
          <span>CA$18,420 this quarter</span>
        </div>
        <svg className="balance-chart" viewBox="0 0 420 112" preserveAspectRatio="none" aria-hidden="true">
          <defs>
            <linearGradient id="chartFill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor="#9D7CFF" stopOpacity="0.5" />
              <stop offset="1" stopColor="#9D7CFF" stopOpacity="0" />
            </linearGradient>
            <linearGradient id="chartLine" x1="0" y1="0" x2="1" y2="0">
              <stop stopColor="#65E9C5" />
              <stop offset="0.55" stopColor="#AE82FF" />
              <stop offset="1" stopColor="#F385DB" />
            </linearGradient>
          </defs>
          <path
            d="M0 95C28 89 44 77 70 81C100 86 114 57 145 62C172 67 185 49 210 51C241 54 253 30 284 38C316 46 337 17 365 24C389 30 401 12 420 9V112H0Z"
            fill="url(#chartFill)"
          />
          <path
            d="M0 95C28 89 44 77 70 81C100 86 114 57 145 62C172 67 185 49 210 51C241 54 253 30 284 38C316 46 337 17 365 24C389 30 401 12 420 9"
            fill="none"
            stroke="url(#chartLine)"
            strokeWidth="3"
            strokeLinecap="round"
          />
          <circle cx="420" cy="9" r="5" fill="#F7D6F3" />
        </svg>
        <div className="balance-card__footer">
          <span>
            <small>In USD</small>
            <strong>$539,420</strong>
          </span>
          <span>
            <small>FX impact</small>
            <strong className="positive">+CA$3,120</strong>
          </span>
        </div>
      </article>

      <article className="preview-card goal-card">
        <div className="preview-card__heading">
          <span>Freedom fund</span>
          <span>68%</span>
        </div>
        <strong>CA$510,000</strong>
        <div className="goal-progress">
          <i />
        </div>
        <small>Across 7 accounts in 3 countries</small>
      </article>
    </div>
  );
}

export default function LandingPage() {
  return (
    <div className="landing-page">
      <a className="skip-link" href="#main-content">
        Skip to content
      </a>

      <header className="landing-nav">
        <a href="#home" aria-label="TrueNorth home">
          <Brand />
        </a>
        <nav aria-label="Main navigation">
          <a href="#platform">Platform</a>
          <a href="#why-truenorth">Why TrueNorth</a>
          <a href="#privacy">Privacy</a>
          <a href={REPOSITORY_URL} target="_blank" rel="noreferrer">
            Roadmap
          </a>
        </nav>
        <a className="nav-cta" href={RELEASES_URL} target="_blank" rel="noreferrer">
          Get early access
          <ArrowIcon />
        </a>
      </header>

      <main id="main-content">
        <section className="hero" id="home">
          <div className="hero-frame">
            <div className="hero-frame__noise" />
            <div className="hero-copy">
              <div className="hero-eyebrow">
                <i />
                Private beta
                <span>Canada + US first</span>
              </div>
              <h1>
                Your wealth crossed borders.
                <span>Your financial picture should too.</span>
              </h1>
              <p>
                TrueNorth brings every account, currency, and long-term goal into one private view
                built for people whose financial lives span countries.
              </p>
              <div className="hero-actions">
                <a className="primary-cta" href="#platform">
                  Explore the platform
                  <span>
                    <ArrowIcon />
                  </span>
                </a>
                <a className="secondary-cta" href="#why-truenorth">
                  Why TrueNorth
                </a>
              </div>
              <ul className="hero-proof" aria-label="TrueNorth principles">
                <li>
                  <LockIcon />
                  Local-first
                </li>
                <li>
                  <GlobeIcon />
                  Multi-currency
                </li>
                <li>
                  <ChartIcon />
                  Read-only
                </li>
              </ul>
            </div>
            <WealthPreview />
          </div>
        </section>

        <section className="signal-strip" aria-label="Core product capabilities">
          <span>One net worth</span>
          <i />
          <span>Every currency</span>
          <i />
          <span>Across borders</span>
          <i />
          <span>Private by default</span>
        </section>

        <section className="why-section" id="why-truenorth">
          <div className="section-heading">
            <div>
              <span className="section-kicker">Why TrueNorth</span>
              <h2>Built for the finances ordinary apps leave behind.</h2>
            </div>
            <p>
              Most finance tools assume one country, one currency, and one financial system.
              TrueNorth starts where they stop.
            </p>
          </div>

          <div className="feature-grid">
            {features.map((feature) => (
              <article className="feature-card" key={feature.number}>
                <div className="feature-card__topline">
                  <span className="feature-icon">{feature.icon}</span>
                  <span>{feature.number}</span>
                </div>
                <h3>{feature.title}</h3>
                <p>{feature.description}</p>
              </article>
            ))}
          </div>
        </section>

        <section className="platform-section" id="platform">
          <div className="platform-copy">
            <span className="section-kicker">One financial home</span>
            <h2>See what you own—not where it happens to live.</h2>
            <p>
              TrueNorth normalizes balances without erasing their context. Keep the original
              currency, understand the conversion, and choose the home currency that guides your
              decisions.
            </p>
            <ul className="platform-list">
              <li>
                <span>01</span>
                Banks and brokerages together
              </li>
              <li>
                <span>02</span>
                Original and home-currency values
              </li>
              <li>
                <span>03</span>
                Progress separated from FX movement
              </li>
            </ul>
            <a href={REPOSITORY_URL} target="_blank" rel="noreferrer">
              Follow the public build
              <ArrowIcon />
            </a>
          </div>

          <div className="account-ledger" aria-label="Illustrative connected accounts">
            <div className="account-ledger__glow" />
            <div className="account-ledger__header">
              <span>All accounts</span>
              <div aria-label="Illustrative display currency">
                <span className="active">CAD</span>
                <span>USD</span>
              </div>
            </div>
            <div className="ledger-total">
              <small>Global net worth</small>
              <strong>CA$742,580</strong>
              <span>Updated across three regions</span>
            </div>
            <div className="ledger-accounts">
              <div>
                <i className="flag-token flag-token--ca">CA</i>
                <span>
                  <strong>Canadian retirement</strong>
                  <small>RRSP · 2 accounts</small>
                </span>
                <b>CA$314,560</b>
              </div>
              <div>
                <i className="flag-token flag-token--us">US</i>
                <span>
                  <strong>US investments</strong>
                  <small>Brokerage · USD</small>
                </span>
                <b>CA$259,210</b>
              </div>
              <div>
                <i className="flag-token flag-token--jm">JM</i>
                <span>
                  <strong>Jamaican accounts</strong>
                  <small>Savings · JMD</small>
                </span>
                <b>CA$86,430</b>
              </div>
              <div>
                <i className="flag-token flag-token--other">+</i>
                <span>
                  <strong>Property & other</strong>
                  <small>Manual assets</small>
                </span>
                <b>CA$82,380</b>
              </div>
            </div>
            <div className="ledger-fx">
              <span>
                <GlobeIcon />
                Today&apos;s FX context
              </span>
              <strong>1 USD = 1.377 CAD</strong>
            </div>
          </div>
        </section>

        <section className="privacy-section" id="privacy">
          <div className="privacy-orbit" aria-hidden="true">
            <span />
            <span />
            <BrandMark className="privacy-orbit__mark" />
          </div>
          <div className="privacy-copy">
            <span className="section-kicker">Privacy is the architecture</span>
            <h2>Your whole financial life should not become someone else&apos;s dataset.</h2>
            <p>
              TrueNorth is local-first, encrypted at rest, and designed around read-only access.
              You decide what connects, what the advisor can see, and when anything leaves your
              device.
            </p>
            <div className="privacy-points">
              <span>
                <LockIcon />
                Encrypted local database
              </span>
              <span>
                <ChartIcon />
                Read-only connections
              </span>
              <span>
                <SparkIcon />
                Optional private AI
              </span>
            </div>
          </div>
        </section>

        <section className="closing-section">
          <div className="closing-section__glow" />
          <BrandMark className="closing-mark" />
          <span className="section-kicker">Find your true north</span>
          <h2>One life. Many countries. One financial picture.</h2>
          <p>
            TrueNorth is starting with cross-border households in Canada and the United States,
            then expanding wherever globally mobile lives lead.
          </p>
          <div className="hero-actions">
            <a className="primary-cta" href={RELEASES_URL} target="_blank" rel="noreferrer">
              Get early access
              <span>
                <ArrowIcon />
              </span>
            </a>
            <a className="secondary-cta" href={REPOSITORY_URL} target="_blank" rel="noreferrer">
              View on GitHub
            </a>
          </div>
        </section>
      </main>

      <footer className="landing-footer">
        <a href="#home" aria-label="Back to the TrueNorth home section">
          <Brand />
        </a>
        <p>Private, cross-border wealth clarity.</p>
        <div>
          <a href={REPOSITORY_URL} target="_blank" rel="noreferrer">
            GitHub
          </a>
          <a href="#privacy">Privacy</a>
          <a href="#home">Back to top ↑</a>
        </div>
      </footer>
    </div>
  );
}
