"use client";

import Link from "next/link";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Skill } from "../lib/catalog";
import { SCORE_EXPLAINER, formatCount, scoreBand, scoreBar, skillPath } from "../lib/catalog";

const SORT_NAMES = {
  score: "skillrank score",
  stars: "stars",
  lift: "eval lift"
} as const;

const SUBMIT_URL =
  "https://github.com/buildbetter-app/skillrank/issues/new?template=skill-submission.yml";

type SortKey = keyof typeof SORT_NAMES;

type RegistryClientProps = {
  skills: Skill[];
};

function sortValue(skill: Skill, sort: SortKey) {
  if (sort === "stars") return skill.signals.stars ?? -1;
  if (sort === "lift") return skill.eval.success_delta_pct ?? -Infinity;
  return skill.score;
}

function searchableText(skill: Skill) {
  return [
    skill.slug,
    skill.display_name,
    skill.category,
    skill.tags.join(" "),
    skill.description,
    skill.source_repo
  ]
    .join(" ")
    .toLowerCase();
}

function categoryCounts(skills: Skill[], query: string) {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const matching = words.length
    ? skills.filter((skill) => words.every((word) => searchableText(skill).includes(word)))
    : skills;
  const counts = new Map<string, number>();
  matching.forEach((skill) => counts.set(skill.category, (counts.get(skill.category) ?? 0) + 1));
  const categories = [...new Set(skills.map((skill) => skill.category))].sort();
  return {
    all: matching.length,
    categories: categories.map((name) => [name, counts.get(name) ?? 0] as const)
  };
}

function copyText(button: HTMLButtonElement, text: string) {
  const old = button.textContent ?? "copy";
  const done = () => {
    button.textContent = "copied ✓";
    window.setTimeout(() => {
      button.textContent = old;
    }, 1300);
  };

  if (navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(text).then(done, done);
  } else {
    done();
  }
}

export function RegistryClient({ skills }: RegistryClientProps) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("all");
  const [sort, setSort] = useState<SortKey>("score");
  const [selected, setSelected] = useState(0);
  const [openSlug, setOpenSlug] = useState<string | null>(null);
  const [themeLabel, setThemeLabel] = useState("◐ theme");
  const searchRef = useRef<HTMLInputElement>(null);
  const rowsRef = useRef<HTMLDivElement>(null);

  const counts = useMemo(() => categoryCounts(skills, query), [skills, query]);

  const filtered = useMemo(() => {
    const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return skills
      .filter((skill) => {
        if (category !== "all" && skill.category !== category) return false;
        if (!words.length) return true;
        const haystack = searchableText(skill);
        return words.every((word) => haystack.includes(word));
      })
      .sort((a, b) => {
        const primary = sortValue(b, sort) - sortValue(a, sort);
        if (primary !== 0) return primary;
        return a.slug.localeCompare(b.slug);
      });
  }, [category, query, skills, sort]);

  useEffect(() => {
    setSelected((current) => Math.max(0, Math.min(current, Math.max(filtered.length - 1, 0))));
  }, [filtered.length]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const typing = document.activeElement === searchRef.current;
      if (event.key === "/" && !typing) {
        event.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
      }

      if (typing) {
        if (event.key === "Escape") {
          event.preventDefault();
          setQuery("");
          setSelected(0);
          searchRef.current?.blur();
        }
        return;
      }

      if (event.key === "ArrowDown" || event.key === "j") {
        event.preventDefault();
        setSelected((current) => {
          const next = Math.min(filtered.length - 1, current + 1);
          rowsRef.current?.querySelectorAll(".row")[next]?.scrollIntoView({ block: "nearest" });
          return next;
        });
      } else if (event.key === "ArrowUp" || event.key === "k") {
        event.preventDefault();
        setSelected((current) => {
          const next = Math.max(0, current - 1);
          rowsRef.current?.querySelectorAll(".row")[next]?.scrollIntoView({ block: "nearest" });
          return next;
        });
      } else if (event.key === "Enter") {
        const skill = filtered[selected];
        if (skill) {
          event.preventDefault();
          setOpenSlug((current) => (current === skill.slug ? null : skill.slug));
          rowsRef.current?.querySelectorAll(".row")[selected]?.scrollIntoView({ block: "nearest" });
        }
      } else if (event.key.toLowerCase() === "g") {
        const skill = filtered[selected];
        if (skill) {
          window.location.href = skill.source_url;
        }
      } else if (event.key.toLowerCase() === "i") {
        document.getElementById("top")?.scrollIntoView({
          behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth"
        });
      } else if (event.key.toLowerCase() === "s") {
        window.location.href = SUBMIT_URL;
      }
    };

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [filtered, selected]);

  function changeTheme() {
    const root = document.documentElement;
    let current = root.getAttribute("data-theme");
    if (!current) {
      current = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    }
    const next = current === "dark" ? "light" : "dark";
    root.setAttribute("data-theme", next);
    setThemeLabel(next === "dark" ? "dark crt" : "light print");
  }

  return (
    <>
      <header className="topbar">
        <span className="brand">
          skillrank<span className="cur">_</span>
        </span>
        <span className="kv hidesm">
          registry <i>▸</i> <span>{skills.length}</span> skills
        </span>
        <span className="kv hidesm">
          evals <i>▸</i> <span>0</span> trials
        </span>
        <span className="kv hidesm">
          agents <i>▸</i> claude · codex
        </span>
        <span className="sp" />
        <span className="kv">MIT · open registry</span>
      </header>

      <main className="wrap">
        <section className="hero" id="top">
          <p className="tag">the registry &amp; benchmark for agent skills</p>
          <h1>
            Find the skills that make your coding agent <b>measurably</b> faster -- install one, or make
            skillrank ambient in Claude Code &amp; Codex.
          </h1>
          <div className="installrow">
            <code>
              <span className="p">$</span> curl -fsSL <b>skillrank.dev</b> | sh
            </code>
            <button
              className="copy"
              type="button"
              onClick={(event) => copyText(event.currentTarget, "curl -fsSL skillrank.dev | sh")}
            >
              copy
            </button>
          </div>
          <p className="aux">
            <span>
              <span className="c"># ambient over MCP →</span> <code>skillrank setup</code>
            </span>
            <span>
              <span className="c"># just find one →</span> <code>skillrank recommend</code>
            </span>
            <span className="c">no account to browse · no telemetry</span>
          </p>
        </section>

        <div className="reg-head">
          <label className="search">
            <span className="pfx">
              <b>$</b> skillrank search
            </span>
            <input
              ref={searchRef}
              value={query}
              type="text"
              placeholder="react, testing, tokens, playwright..."
              autoComplete="off"
              spellCheck="false"
              aria-label="Search skills"
              onChange={(event) => {
                setQuery(event.target.value);
                setSelected(0);
              }}
            />
            <span className="caret" aria-hidden="true">
              ▊
            </span>
            <kbd>/</kbd>
          </label>
        </div>

        <div className="registry">
          <aside className="side">
            <div className="grp">
              <p className="glabel">sort by</p>
              <div className="opts">
                {(Object.keys(SORT_NAMES) as SortKey[]).map((key) => (
                  <button
                    className="opt"
                    key={key}
                    type="button"
                    aria-pressed={sort === key}
                    onClick={() => {
                      setSort(key);
                      setSelected(0);
                    }}
                  >
                    {SORT_NAMES[key]}
                    {sort === key ? <span className="cnt">▼</span> : null}
                  </button>
                ))}
              </div>
            </div>

            <div className="grp">
              <p className="glabel">category</p>
              <div className="opts">
                <button
                  className="opt"
                  type="button"
                  aria-pressed={category === "all"}
                  onClick={() => {
                    setCategory("all");
                    setSelected(0);
                  }}
                >
                  all <span className="cnt">{counts.all}</span>
                </button>
                {counts.categories.map(([name, count]) => (
                  <button
                    className="opt"
                    key={name}
                    type="button"
                    aria-pressed={category === name}
                    onClick={() => {
                      setCategory(name);
                      setSelected(0);
                    }}
                  >
                    {name} <span className="cnt">{count}</span>
                  </button>
                ))}
              </div>
            </div>

            <p className="legend-note">
              <b>SkillRank score</b> {SCORE_EXPLAINER.replace("SkillRank score ", "")}
            </p>
          </aside>

          <section className="list">
            <div className="listhead">
              <h2>{category === "all" ? (sort === "score" ? "top skills" : "all skills") : `${category} skills`}</h2>
              <span className="meta">
                <b>{filtered.length}</b> of {skills.length} · sorted by <b>{SORT_NAMES[sort]}</b>
              </span>
            </div>
            <div className="colhead">
              <span className="r">#</span>
              <span>skill</span>
              <span className="r c-star">★ stars</span>
              <span className="r c-tier">tier</span>
              <span className="r">Δ success</span>
              <span className="r">score</span>
            </div>
            <div className="rows" ref={rowsRef}>
              {filtered.length ? (
                filtered.map((skill, index) => {
                  const isOpen = openSlug === skill.slug;
                  const isSelected = selected === index;
                  return (
                    <div
                      className={`row ${index < 3 ? `top${index + 1}` : ""} ${isSelected ? "sel" : ""} ${
                        isOpen ? "open" : ""
                      }`}
                      key={skill.slug}
                    >
                      <button
                        className="row-main"
                        type="button"
                        onClick={() => {
                          setSelected(index);
                          setOpenSlug(isOpen ? null : skill.slug);
                        }}
                        aria-expanded={isOpen}
                      >
                        <span className="rank">{index + 1}</span>
                        <span className="nm">
                          <span className="n">
                            <span className="caret">▸</span>
                            {skill.slug}
                          </span>
                          <span className="sub">
                            <span className="cat">{skill.category}</span> · {skill.tags.join(" · ")}
                          </span>
                        </span>
                        <span className="num star c-star">★ {formatCount(skill.signals.stars)}</span>
                        <span className={`tier c-tier ${skill.eval.tier === "official" ? "official" : ""}`}>
                          {skill.eval.tier}
                        </span>
                        <span className="num succ">
                          <span className="pend">eval pending</span>
                        </span>
                        <span className={`score ${scoreBand(skill.score)}`}>
                          <span className="bar">{scoreBar(skill.score)}</span>
                          <span className="v">{skill.score}</span>
                        </span>
                      </button>
                      <div className="detail">
                        <p className="desc">{skill.description}</p>
                        <div className="signals">
                          <div className="sig">
                            <div className="lab">community</div>
                            <div className="val">★ {formatCount(skill.signals.stars)}</div>
                          </div>
                          <div className="sig">
                            <div className="lab">usage</div>
                            <div className="val">
                              {formatCount(skill.signals.installs)} <small>installs</small>
                            </div>
                          </div>
                          <div className="sig">
                            <div className="lab">eval lift</div>
                            <div className="val">
                              <small>eval pending</small>
                            </div>
                          </div>
                        </div>
                        <div className="instrow">
                          <code>
                            <span className="p">$</span> skillrank install <b>{skill.slug}</b>
                          </code>
                          <button
                            className="copy"
                            type="button"
                            onClick={(event) => copyText(event.currentTarget, `skillrank install ${skill.slug}`)}
                          >
                            copy
                          </button>
                        </div>
                        <p className="dmeta">
                          <span className={`tier ${skill.eval.tier === "official" ? "official" : ""}`}>
                            {skill.eval.tier}
                          </span>
                          <span className="badge">provisional</span>
                          <span>
                            skillrank score <b>{skill.score}</b>/100
                          </span>
                          <a href={skill.source_url}>source ▸</a>
                          <Link href={skillPath(skill.slug)}>view full page ▸</Link>
                        </p>
                        <p className="explain">{SCORE_EXPLAINER}</p>
                      </div>
                    </div>
                  );
                })
              ) : (
                <div className="empty">
                  no skills match <b>&quot;{query}&quot;</b> -- try a stack, a tag, or clear the search.
                </div>
              )}
            </div>
          </section>
        </div>

        <footer className="foot">
          <span>
            <b>skillrank</b> -- MIT
          </span>
          <span>github.com/buildbetter-app/skillrank</span>
          <span>
            <a href={SUBMIT_URL}>submit a skill ▸</a>
          </span>
          <span>part of the BuildBetter universe</span>
        </footer>
      </main>

      <nav className="keybar" aria-label="commands">
        <a className="k" href="#top">
          <kbd>i</kbd>install
        </a>
        <button className="k keylink" type="button" onClick={() => searchRef.current?.focus()}>
          <kbd>/</kbd>search
        </button>
        <span className="k hidesm">
          <kbd>↑↓</kbd>move
        </span>
        <span className="k hidesm">
          <kbd>↵</kbd>open
        </span>
        <span className="k">
          <kbd>g</kbd>github
        </span>
        <a className="k" href={SUBMIT_URL}>
          <kbd>s</kbd>submit
        </a>
        <span className="sp" />
        <button className="themebtn" type="button" onClick={changeTheme}>
          {themeLabel}
        </button>
      </nav>
    </>
  );
}
