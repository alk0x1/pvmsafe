import {
  HashRouter,
  Link,
  NavLink,
  Route,
  Routes,
} from "react-router-dom";
import { Home } from "./pages/Home";
import { Docs } from "./pages/Docs";
import { Demo } from "./pages/Demo";

function TopNav() {
  return (
    <header className="site-top">
      <Link to="/" className="brand">
        <span className="brand-mark">pvmsafe</span>
        <span className="brand-sub">compile-time verification · pallet-revive</span>
      </Link>
      <nav className="site-nav">
        <NavLink end to="/" className={({ isActive }) => (isActive ? "active" : "")}>
          Home
        </NavLink>
        <NavLink to="/docs" className={({ isActive }) => (isActive ? "active" : "")}>
          Docs
        </NavLink>
        <NavLink to="/demo" className={({ isActive }) => (isActive ? "active" : "")}>
          Demo
        </NavLink>
      </nav>
    </header>
  );
}

function SiteFooter() {
  return (
    <footer className="site-footer">
      <span>pvmsafe · a Rust proc-macro crate for pallet-revive</span>
      <span className="footer-sep">·</span>
      <a href="https://github.com/paritytech/cargo-pvm-contract" target="_blank" rel="noreferrer">
        cargo-pvm-contract
      </a>
    </footer>
  );
}

export function App() {
  return (
    <HashRouter>
      <div className="site">
        <TopNav />
        <main className="site-main">
          <Routes>
            <Route path="/" element={<Home />} />
            <Route path="/docs" element={<Docs />} />
            <Route path="/demo" element={<Demo />} />
          </Routes>
        </main>
        <SiteFooter />
      </div>
    </HashRouter>
  );
}
