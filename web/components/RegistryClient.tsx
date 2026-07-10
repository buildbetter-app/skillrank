"use client";

import Link from "next/link";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Skill } from "../lib/catalog";
import { SCORE_EXPLAINER, formatCount, scoreBand, scoreBar, skillPath } from "../lib/catalog";

const SORT_NAMES = {
  score: "skillrank score",
  stars: "stars",
  skills: "skills"
} as const;

const SUBMIT_URL =
  "https://github.com/buildbetter-app/skillrank/issues/new?template=skill-submission.yml";

type SortKey = keyof typeof SORT_NAMES;

type RegistryClientProps = {
  skills: Skill[];
};

type RepoGroup = {
  repo: string;
  stars: number | null;
  score: number;
  tier: Skill["eval"]["tier"];
  sourceUrl: string;
  skills: Skill[];
  categories: Set<string>;
};

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

function repoTags(group: RepoGroup) {
  return [...new Set(group.skills.flatMap((skill) => skill.tags))].slice(0, 2);
}

function repoCategoryLabel(group: RepoGroup) {
  return group.categories.size === 1 ? [...group.categories][0] : "mixed";
}

function sortGroups(groups: RepoGroup[], sort: SortKey) {
  return [...groups].sort((a, b) => {
    let primary = 0;
    if (sort === "stars") {
      primary = (b.stars ?? -1) - (a.stars ?? -1);
    } else if (sort === "skills") {
      primary = b.skills.length - a.skills.length;
    } else {
      primary = b.score - a.score;
    }
    if (primary !== 0) return primary;
    return a.repo.localeCompare(b.repo);
  });
}

export function RegistryClient({ skills }: RegistryClientProps) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("all");
  const [sort, setSort] = useState<SortKey>("score");
  const [selected, setSelected] = useState(0);
  const [openRepos, setOpenRepos] = useState<Set<string>>(new Set());
  const [limit, setLimit] = useState(60); // cap rendered repo rows; "load more" extends
  const [themeLabel, setThemeLabel] = useState("◐ theme");
  const searchRef = useRef<HTMLInputElement>(null);
  const rowsRef = useRef<HTMLDivElement>(null);

  const counts = useMemo(() => categoryCounts(skills, query), [skills, query]);
  const hasQuery = query.trim().length > 0;

  const filteredSkills = useMemo(() => {
    const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return skills
      .filter((skill) => {
        if (category !== "all" && skill.category !== category) return false;
        if (!words.length) return true;
        const haystack = searchableText(skill);
        return words.every((word) => haystack.includes(word));
      });
  }, [category, query, skills]);

  const groups = useMemo(() => {
    const byRepo = new Map<string, RepoGroup>();
    filteredSkills.forEach((skill) => {
      const existing = byRepo.get(skill.source_repo);
      if (existing) {
        existing.skills.push(skill);
        existing.categories.add(skill.category);
        return;
      }
      byRepo.set(skill.source_repo, {
        repo: skill.source_repo,
        stars: skill.signals.stars,
        score: skill.score,
        tier: skill.eval.tier,
        sourceUrl: skill.source_url,
        skills: [skill],
        categories: new Set([skill.category])
      });
    });
    const repoGroups = [...byRepo.values()].map((group) => ({
      ...group,
      skills: [...group.skills].sort((a, b) => a.slug.localeCompare(b.slug))
    }));
    return sortGroups(repoGroups, sort);
  }, [filteredSkills, sort]);

  useEffect(() => {
    setSelected((current) => Math.max(0, Math.min(current, Math.max(groups.length - 1, 0))));
  }, [groups.length]);

  // reset the render window whenever the result set changes
  useEffect(() => setLimit(60), [query, category, sort]);

  useEffect(() => {
    setOpenRepos(new Set());
  }, [query, category, sort]);

  function toggleRepo(repo: string) {
    setOpenRepos((current) => {
      const next = new Set(current);
      if (next.has(repo)) {
        next.delete(repo);
      } else {
        next.add(repo);
      }
      return next;
    });
  }

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
        if (!groups.length) return;
        setSelected((current) => {
          const next = Math.min(groups.length - 1, current + 1);
          setLimit((l) => (next >= l ? next + 40 : l)); // pull more repo rows into view when arrowing past the window
          requestAnimationFrame(() =>
            rowsRef.current?.querySelectorAll(".row")[next]?.scrollIntoView({ block: "nearest" })
          );
          return next;
        });
      } else if (event.key === "ArrowUp" || event.key === "k") {
        event.preventDefault();
        if (!groups.length) return;
        setSelected((current) => {
          const next = Math.max(0, current - 1);
          rowsRef.current?.querySelectorAll(".row")[next]?.scrollIntoView({ block: "nearest" });
          return next;
        });
      } else if (event.key === "Enter") {
        const group = groups[selected];
        if (group) {
          event.preventDefault();
          toggleRepo(group.repo);
          rowsRef.current?.querySelectorAll(".row")[selected]?.scrollIntoView({ block: "nearest" });
        }
      } else if (event.key.toLowerCase() === "g") {
        const group = groups[selected];
        if (group) {
          window.location.href = group.sourceUrl;
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
  }, [groups, selected]);

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
            Find the skills that make your coding agent <b>measurably</b>{" "}
            faster -- install one, or make skillrank ambient in Claude Code &amp; Codex.
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
              <b>SkillRank score</b> {SCORE_EXPLAINER.replace("SkillRank score ", "")}{" "}
              <Link href="/how-it-works">how it works ▸</Link>
            </p>
          </aside>

          <section className="list">
            <div className="listhead">
              <h2>{category === "all" ? (sort === "score" ? "top repos" : "all repos") : `${category} repos`}</h2>
              <span className="meta">
                <b>{groups.length}</b> repos · <b>{filteredSkills.length}</b> skills · sorted by{" "}
                <b>{SORT_NAMES[sort]}</b>
              </span>
            </div>
            <div className="colhead">
              <span className="r">#</span>
              <span>repo</span>
              <span className="r c-star">★ stars</span>
              <span className="r c-tier">tier</span>
              <span className="r">skills</span>
              <span className="r">score</span>
            </div>
            <div className="rows" ref={rowsRef}>
              {groups.length ? (
                <>
                  {groups.slice(0, limit).map((group, index) => {
                    const isOpen = hasQuery || openRepos.has(group.repo);
                    const isSelected = selected === index;
                    const tags = repoTags(group);
                    const subParts = [
                      `${group.skills.length} ${group.skills.length === 1 ? "skill" : "skills"}`,
                      repoCategoryLabel(group),
                      ...tags
                    ];
                    return (
                      <div
                        className={`row ${index < 3 ? `top${index + 1}` : ""} ${isSelected ? "sel" : ""} ${
                          isOpen ? "open" : ""
                        }`}
                        key={group.repo}
                      >
                        <button
                          className="row-main"
                          type="button"
                          onClick={() => {
                            setSelected(index);
                            toggleRepo(group.repo);
                          }}
                          aria-expanded={isOpen}
                        >
                          <span className="rank">{index + 1}</span>
                          <span className="nm">
                            <span className="n">
                              <span className="caret">▸</span>
                              {group.repo}
                            </span>
                            <span className="sub">
                              {subParts.map((part, partIndex) => (
                                <span className={partIndex === 1 ? "cat" : undefined} key={`${group.repo}-${part}`}>
                                  {partIndex > 0 ? " · " : ""}
                                  {part}
                                </span>
                              ))}
                            </span>
                          </span>
                          <span className="num star c-star">★ {formatCount(group.stars)}</span>
                          <span className={`tier c-tier ${group.tier === "official" ? "official" : ""}`}>
                            {group.tier}
                          </span>
                          <span className="num succ">{group.skills.length}</span>
                          <span className={`score ${scoreBand(group.score)}`}>
                            <span className="bar">{scoreBar(group.score)}</span>
                            <span className="v">{group.score}</span>
                          </span>
                        </button>
                        <div className="detail repo-detail">
                          {isOpen ? group.skills.map((skill) => (
                            <div className="skillrow" key={skill.slug}>
                              <div className="skillrow-head">
                                <span className="skillrow-name">{skill.display_name}</span>
                                <span className="skillrow-slug">{skill.slug}</span>
                              </div>
                              <p className="desc">{skill.description}</p>
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
                                {skill.eval.token_delta_pct != null ? (
                                  <span className={`badge eval ${skill.eval.token_delta_pct <= 0 ? "good" : "bad"}`}>
                                    tokens {skill.eval.token_delta_pct > 0 ? "+" : ""}
                                    {skill.eval.token_delta_pct}% · success{" "}
                                    {(skill.eval.success_delta_pct ?? 0) > 0 ? "+" : ""}
                                    {skill.eval.success_delta_pct}pp (n={skill.eval.n_trials})
                                  </span>
                                ) : (
                                  <span className="badge">eval pending</span>
                                )}
                                <Link href={skillPath(skill.slug)}>view page ▸</Link>
                              </p>
                            </div>
                          )) : null}
                        </div>
                      </div>
                    );
                  })}
                  {groups.length > limit ? (
                    <button className="loadmore" type="button" onClick={() => setLimit((l) => l + 80)}>
                      load {Math.min(80, groups.length - limit)} more · {groups.length - limit} remaining
                    </button>
                  ) : null}
                </>
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
            <Link href="/how-it-works">how evals work ▸</Link>
          </span>
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
