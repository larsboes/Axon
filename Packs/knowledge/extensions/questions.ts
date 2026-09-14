/**
 * Questions for the human in the loop — one widget family, two tools.
 *
 * `ask` — a decision round. Each question carries 2-4 mutually exclusive options (the schema allows
 * up to 9), a one-line consequence per option, an optional preview block rendered only for the
 * highlighted option, and your recommendation marked and focused by default. The user picks one,
 * then may attach a free-text note to that decision. Returns per-decision choices WITH the notes, so
 * the caller can tell an informed answer from an accepted default, and can write the reasoning down.
 *
 * `quiz` — a graded multiple-choice quiz. The model supplies the correct answer and the
 * explanation; the widget grades on selection and returns correct / incorrect / IDK per question.
 *
 * Used by the `teach` skill (probe + understanding checks) and by question rounds in
 * `crystallize`.
 *
 * Non-interactive modes (-p, json, rpc-without-tui): both tools return a graceful error instead
 * of hanging — the model should fall back to asking in plain text.
 */

import type { ExtensionAPI, ExtensionContext, Theme } from "@earendil-works/pi-coding-agent";
import {
	Editor,
	type EditorTheme,
	Key,
	matchesKey,
	Text,
	visibleWidth,
	wrapTextWithAnsi,
} from "@earendil-works/pi-tui";
import { Type } from "typebox";

/* ────────────────────────────── shared ────────────────────────────── */

/** Wraps `text` and prefixes the first line, indenting the continuation lines to match. */
function addPrefixed(lines: string[], width: number, prefix: string, text: string) {
	const pw = visibleWidth(prefix);
	if (pw >= width) {
		lines.push(...wrapTextWithAnsi(prefix + text, Math.max(1, width)));
		return;
	}
	const wrapped = wrapTextWithAnsi(text, Math.max(1, width - pw));
	const cont = " ".repeat(pw);
	for (let i = 0; i < wrapped.length; i++) lines.push(`${i === 0 ? prefix : cont}${wrapped[i]}`);
}

function editorTheme(theme: Theme): EditorTheme {
	return {
		borderColor: (s) => theme.fg("border", s),
		selectList: {
			selectedPrefix: (t) => theme.fg("accent", t),
			selectedText: (t) => theme.fg("accent", t),
			description: (t) => theme.fg("muted", t),
			scrollInfo: (t) => theme.fg("dim", t),
			noMatch: (t) => theme.fg("warning", t),
		},
	};
}

/** Both tools answer with this when there is no terminal to ask in. */
const NON_TUI_ERROR = "Error: interactive UI unavailable (non-interactive mode).";

/* ────────────────────────── ask: a decision round ────────────────────────── */

interface AskOption {
	label: string;
	consequence?: string;
	preview?: string;
}

interface AskQuestion {
	id: string;
	title?: string;
	prompt: string;
	context?: string;
	options: AskOption[];
	recommendedIndex?: number;
	allowNote?: boolean;
}

interface AskAnswer {
	id: string;
	title?: string;
	/** null when the question was skipped. */
	selectedIndex: number | null;
	label: string | null;
	/** Free text the user attached to this decision, or null. */
	note: string | null;
	/** null when there was no recommendation, or the question was skipped. */
	followedRecommendation: boolean | null;
	skipped: boolean;
}

interface AskResult {
	topic: string;
	answers: AskAnswer[];
	cancelled: boolean;
}

const AskParams = Type.Object({
	topic: Type.String({ description: "Round label, e.g. 'Round 3 — vault structure'" }),
	questions: Type.Array(
		Type.Object({
			id: Type.String({ description: "Short id, e.g. 'q1' or 'vault-root'" }),
			title: Type.Optional(
				Type.String({ description: "Short label for the fork, e.g. 'Data classes'. Shown next to the id." }),
			),
			prompt: Type.String({
				description: "The question itself, with the evidence that makes it a real fork.",
			}),
			context: Type.Optional(
				Type.String({
					description:
						"Evidence paragraph shown dim above the options — what you measured, what disagrees, where it lives.",
				}),
			),
			options: Type.Array(
				Type.Object({
					label: Type.String({ description: "The option, in a few words." }),
					consequence: Type.Optional(
						Type.String({
							description:
								"What happens if this is chosen — one line, and include the cost. Not a restatement of the option.",
						}),
					),
					preview: Type.Optional(
						Type.String({
							description:
								"Block rendered only while this option is highlighted: folder layout, config, data model. Separate lines with \\n.",
						}),
					),
				}),
				{ description: "2-4 mutually exclusive options (up to 9). Put the one you recommend first." },
			),
			recommendedIndex: Type.Optional(
				Type.Number({ description: "0-based index of the option you recommend. Marked and focused by default." }),
			),
			allowNote: Type.Optional(
				Type.Boolean({ description: "Whether the user can attach a note after choosing (default true)." }),
			),
		}),
		{ description: "The questions in this round, in order" },
	),
});

function askError(message: string, details: AskResult): { content: { type: "text"; text: string }[]; details: AskResult } {
	return { content: [{ type: "text", text: message }], details };
}

async function runAsk(
	params: { topic: string; questions: AskQuestion[] },
	ctx: ExtensionContext,
): Promise<{ content: { type: "text"; text: string }[]; details: AskResult }> {
	const { topic } = params;
	const questions = params.questions as AskQuestion[];
	const empty: AskResult = { topic, answers: [], cancelled: true };

	if (ctx.mode !== "tui") {
		return askError(
			`${NON_TUI_ERROR} Ask the round as a numbered list in text: 2-9 options each, your recommendation marked, and say a note can be added per question.`,
			empty,
		);
	}
	if (questions.length === 0) return askError("Error: no questions provided", empty);

	const ids = new Set<string>();
	for (const q of questions) {
		if (ids.has(q.id)) throw new Error(`ask: duplicate question id "${q.id}"`);
		ids.add(q.id);
		if (q.options.length < 2 || q.options.length > 9) {
			throw new Error(`ask: question "${q.id}" has ${q.options.length} options — give it 2-9`);
		}
		if (q.recommendedIndex !== undefined && (q.recommendedIndex < 0 || q.recommendedIndex >= q.options.length)) {
			throw new Error(
				`ask: question "${q.id}" recommends option ${q.recommendedIndex}, out of range for ${q.options.length} options`,
			);
		}
	}

	const result = await ctx.ui.custom<AskResult>((tui, theme, _kb, done) => {
		let qIndex = 0;
		let optionIndex = questions[0].recommendedIndex ?? 0;
		let phase: "options" | "note" = "options";
		let cachedLines: string[] | undefined;
		const answers: AskAnswer[] = [];
		const editor = new Editor(tui, editorTheme(theme));

		function refresh() {
			cachedLines = undefined;
			tui.requestRender();
		}

		function record(note: string | null) {
			const q = questions[qIndex];
			answers.push({
				id: q.id,
				title: q.title,
				selectedIndex: optionIndex,
				label: q.options[optionIndex].label,
				note: note && note.length > 0 ? note : null,
				followedRecommendation: q.recommendedIndex === undefined ? null : q.recommendedIndex === optionIndex,
				skipped: false,
			});
		}

		function advance() {
			editor.setText("");
			phase = "options";
			if (qIndex < questions.length - 1) {
				qIndex++;
				optionIndex = questions[qIndex].recommendedIndex ?? 0;
				refresh();
				return;
			}
			done({ topic, answers, cancelled: false });
		}

		function skip() {
			const q = questions[qIndex];
			answers.push({
				id: q.id,
				title: q.title,
				selectedIndex: null,
				label: null,
				note: null,
				followedRecommendation: null,
				skipped: true,
			});
			advance();
		}

		editor.onChange = () => refresh();
		editor.onSubmit = (value) => {
			record(value.trim());
			advance();
		};

		function handleInput(data: string) {
			if (phase === "note") {
				if (matchesKey(data, Key.escape)) {
					editor.setText("");
					phase = "options";
					refresh();
					return;
				}
				editor.handleInput(data);
				refresh();
				return;
			}

			if (matchesKey(data, Key.escape)) {
				done({ topic, answers, cancelled: true });
				return;
			}

			const q = questions[qIndex];
			if (matchesKey(data, Key.up) || data === "k") {
				optionIndex = Math.max(0, optionIndex - 1);
				refresh();
				return;
			}
			if (matchesKey(data, Key.down) || data === "j") {
				optionIndex = Math.min(q.options.length - 1, optionIndex + 1);
				refresh();
				return;
			}
			if (data === "s") {
				skip();
				return;
			}
			// Number keys move focus rather than answering: every option has a consequence worth
			// reading, and the recommendation is already focused, so Enter alone is the fast path.
			const n = Number.parseInt(data, 10);
			if (!Number.isNaN(n) && data === String(n) && n >= 1 && n <= q.options.length) {
				optionIndex = n - 1;
				refresh();
				return;
			}
			if (matchesKey(data, Key.enter)) {
				if (q.allowNote === false) {
					record(null);
					advance();
					return;
				}
				phase = "note";
				refresh();
			}
		}

		function render(width: number): string[] {
			if (cachedLines) return cachedLines;
			const w = Math.max(1, width);
			const lines: string[] = [];
			const q = questions[qIndex];
			const chosen = phase === "note" ? optionIndex : null;

			lines.push(theme.fg("accent", "─".repeat(w)));
			addPrefixed(
				lines,
				w,
				" ",
				theme.fg("muted", `${topic} · ${qIndex + 1}/${questions.length}${q.title ? ` · ${q.title}` : ""}`),
			);
			lines.push("");
			if (q.context) {
				addPrefixed(lines, w, " ", theme.fg("dim", q.context));
				lines.push("");
			}
			addPrefixed(lines, w, " ", theme.fg("text", theme.bold(q.prompt)));
			lines.push("");

			for (let i = 0; i < q.options.length; i++) {
				const opt = q.options[i];
				const focused = i === optionIndex;
				let line = theme.fg(focused ? "accent" : "text", `${i + 1}. ${opt.label}`);
				if (q.recommendedIndex === i) line += "  " + theme.fg("success", "★ recommended");
				if (chosen === i) line += "  " + theme.fg("success", "◆ you chose this");
				addPrefixed(lines, w, focused && phase === "options" ? theme.fg("accent", "> ") : "  ", line);
				if (opt.consequence) addPrefixed(lines, w, "     ", theme.fg("muted", opt.consequence));
				// Previews are long by nature, so only the option in front of the user carries one.
				if (opt.preview && (focused || chosen === i)) {
					for (const raw of opt.preview.split("\n")) {
						addPrefixed(lines, w, "     │ ", theme.fg("dim", raw));
					}
				}
			}

			lines.push("");
			if (phase === "note") {
				addPrefixed(lines, w, " ", theme.fg("text", "Note for this decision (optional):"));
				for (const line of editor.render(Math.max(1, w - 1))) lines.push(` ${line}`);
				lines.push("");
				addPrefixed(lines, w, " ", theme.fg("dim", "Enter to record · an empty note is fine · Esc to change answer"));
			} else {
				addPrefixed(
					lines,
					w,
					" ",
					theme.fg("dim", "↑↓ move · 1-9 focus · Enter choose · s skip (stays open) · Esc end round"),
				);
			}
			lines.push(theme.fg("accent", "─".repeat(w)));

			cachedLines = lines;
			return lines;
		}

		return {
			get focused() {
				return editor.focused;
			},
			set focused(value: boolean) {
				editor.focused = value;
			},
			render,
			invalidate: () => {
				cachedLines = undefined;
			},
			handleInput,
		};
	});

	const answered = result.answers.filter((a) => !a.skipped);
	const overruled = result.answers.filter((a) => a.followedRecommendation === false).length;
	const header = result.cancelled
		? `Decision round stopped early: ${answered.length}/${questions.length} answered.`
		: `Decision round complete: ${answered.length}/${questions.length} answered${
				overruled > 0 ? `, ${overruled} overruled your recommendation` : ""
			}.`;
	const report = [header];
	for (const a of result.answers) {
		const q = questions.find((x) => x.id === a.id);
		const tag = q?.title ? ` (${q.title})` : "";
		if (a.skipped) {
			report.push(`${a.id}${tag}: skipped — still open`);
			continue;
		}
		let line = `${a.id}${tag}: ${a.label}`;
		if (a.followedRecommendation === false && q?.recommendedIndex !== undefined) {
			line += ` — overruled your recommendation: ${q.options[q.recommendedIndex].label}`;
		}
		report.push(line);
		if (a.note) report.push(`    note: ${a.note.replace(/\s*\n\s*/g, " / ")}`);
	}
	if (result.cancelled && answered.length < questions.length) {
		report.push(`Not asked: ${questions.slice(result.answers.length).map((q) => q.id).join(", ")}`);
	}
	return { content: [{ type: "text" as const, text: report.join("\n") }], details: result };
}

/* ────────────────────────── quiz: graded, right or wrong ────────────────────────── */

interface QuizQuestion {
	id: string;
	prompt: string;
	options: string[];
	correctIndex: number;
	explanation?: string;
}

interface QuizAnswer {
	id: string;
	selectedIndex: number | null; // null = "I don't know"
	correct: boolean;
	idk: boolean;
}

interface QuizResult {
	topic: string;
	answers: QuizAnswer[];
	cancelled: boolean;
}

const QuizParams = Type.Object({
	topic: Type.String({ description: "Short label for what this quiz probes, e.g. 'vector calculus prerequisites'" }),
	questions: Type.Array(
		Type.Object({
			id: Type.String({ description: "Unique id, e.g. 'q1'" }),
			prompt: Type.String({ description: "The question text. LaTeX allowed ($...$); keep it terminal-readable too." }),
			options: Type.Array(Type.String(), {
				description: "2-5 answer options. Do NOT include an 'I don't know' option — the widget adds it.",
			}),
			correctIndex: Type.Number({ description: "0-based index of the correct option" }),
			explanation: Type.Optional(
				Type.String({ description: "1-3 sentence explanation shown after answering. Always provide it." }),
			),
		}),
		{ description: "Questions to ask, in order" },
	),
});

async function runQuiz(
	params: { topic: string; questions: QuizQuestion[] },
	ctx: ExtensionContext,
): Promise<{ content: { type: "text"; text: string }[]; details: QuizResult }> {
	const questions = params.questions as QuizQuestion[];
	const error = (message: string): { content: { type: "text"; text: string }[]; details: QuizResult } => ({
		content: [{ type: "text", text: message }],
		details: { topic: params.topic, answers: [], cancelled: true },
	});

	if (ctx.mode !== "tui") {
		return error(`${NON_TUI_ERROR} Ask the questions as plain numbered text instead.`);
	}
	if (questions.length === 0) return error("Error: no questions provided");
	for (const q of questions) {
		if (q.correctIndex < 0 || q.correctIndex >= q.options.length) {
			throw new Error(`Question ${q.id}: correctIndex ${q.correctIndex} out of range for ${q.options.length} options`);
		}
	}

	const result = await ctx.ui.custom<QuizResult>((tui, theme, _kb, done) => {
		let qIndex = 0;
		let optionIndex = 0;
		let revealed = false;
		let cachedLines: string[] | undefined;
		const answers: QuizAnswer[] = [];

		const IDK = "I don't know";

		function refresh() {
			cachedLines = undefined;
			tui.requestRender();
		}

		function currentOptions(): string[] {
			return [...questions[qIndex].options, IDK];
		}

		function selectCurrent() {
			const q = questions[qIndex];
			const idk = optionIndex === q.options.length;
			answers.push({
				id: q.id,
				selectedIndex: idk ? null : optionIndex,
				correct: !idk && optionIndex === q.correctIndex,
				idk,
			});
			revealed = true;
			refresh();
		}

		function advance() {
			if (qIndex < questions.length - 1) {
				qIndex++;
				optionIndex = 0;
				revealed = false;
				refresh();
			} else {
				done({ topic: params.topic, answers, cancelled: false });
			}
		}

		function handleInput(data: string) {
			if (matchesKey(data, Key.escape)) {
				done({ topic: params.topic, answers, cancelled: true });
				return;
			}
			if (revealed) {
				if (matchesKey(data, Key.enter) || data === " ") advance();
				return;
			}
			const opts = currentOptions();
			if (matchesKey(data, Key.up)) {
				optionIndex = Math.max(0, optionIndex - 1);
				refresh();
				return;
			}
			if (matchesKey(data, Key.down)) {
				optionIndex = Math.min(opts.length - 1, optionIndex + 1);
				refresh();
				return;
			}
			// Number keys 1-9 jump-select
			const n = Number.parseInt(data, 10);
			if (!Number.isNaN(n) && n >= 1 && n <= opts.length) {
				optionIndex = n - 1;
				selectCurrent();
				return;
			}
			if (matchesKey(data, Key.enter)) {
				selectCurrent();
			}
		}

		function render(width: number): string[] {
			if (cachedLines) return cachedLines;
			const lines: string[] = [];
			const w = Math.max(1, width);
			const q = questions[qIndex];
			const opts = currentOptions();
			const answer = answers[answers.length - 1];

			lines.push(theme.fg("accent", "─".repeat(w)));
			const score = answers.filter((a) => a.correct).length;
			addPrefixed(
				lines,
				w,
				" ",
				theme.fg("muted", `${params.topic} · question ${qIndex + 1}/${questions.length} · ${score} correct`),
			);
			lines.push("");
			addPrefixed(lines, w, " ", theme.fg("text", theme.bold(q.prompt)));
			lines.push("");

			for (let i = 0; i < opts.length; i++) {
				const isIdk = i === q.options.length;
				const selected = i === optionIndex;
				let prefix = selected && !revealed ? theme.fg("accent", "> ") : "  ";
				let color: Parameters<Theme["fg"]>[0] = selected && !revealed ? "accent" : isIdk ? "muted" : "text";
				let suffix = "";
				if (revealed && answer) {
					if (i === q.correctIndex) {
						color = "success";
						suffix = "  ✓";
					} else if (i === answer.selectedIndex) {
						color = "error";
						suffix = "  ✗ your answer";
					} else if (answer.idk && isIdk) {
						color = "warning";
						suffix = "  — your answer";
					} else {
						color = "dim";
					}
					prefix = "  ";
				}
				addPrefixed(lines, w, prefix, theme.fg(color, `${i + 1}. ${opts[i]}${suffix}`));
			}

			lines.push("");
			if (revealed && answer) {
				const verdict = answer.correct
					? theme.fg("success", "✓ Correct")
					: answer.idk
						? theme.fg("warning", `— The answer: ${q.options[q.correctIndex]}`)
						: theme.fg("error", `✗ Incorrect — correct: ${q.options[q.correctIndex]}`);
				addPrefixed(lines, w, " ", verdict);
				if (q.explanation) {
					lines.push("");
					addPrefixed(lines, w, " ", theme.fg("muted", q.explanation));
				}
				lines.push("");
				addPrefixed(lines, w, " ", theme.fg("dim", "Enter/Space for next · Esc to stop"));
			} else {
				addPrefixed(lines, w, " ", theme.fg("dim", "↑↓ or 1-9 · Enter to answer · Esc to stop"));
			}
			lines.push(theme.fg("accent", "─".repeat(w)));
			cachedLines = lines;
			return lines;
		}

		return {
			render,
			invalidate: () => {
				cachedLines = undefined;
			},
			handleInput,
		};
	});

	const summary = result.answers.map((a) => {
		const q = questions.find((x) => x.id === a.id);
		const chosen = a.idk ? "I don't know" : (q?.options[a.selectedIndex ?? 0] ?? "?");
		const verdict = a.correct ? "CORRECT" : a.idk ? "IDK" : `INCORRECT (correct: ${q?.options[q.correctIndex]})`;
		return `${a.id}: "${chosen}" — ${verdict}`;
	});
	const score = result.answers.filter((a) => a.correct).length;
	const header = result.cancelled
		? `Quiz stopped early after ${result.answers.length}/${questions.length} question(s).`
		: `Quiz complete: ${score}/${questions.length} correct.`;
	return {
		content: [{ type: "text" as const, text: [header, ...summary].join("\n") }],
		details: result,
	};
}

/* ────────────────────────────── registration ────────────────────────────── */

export default function questions(pi: ExtensionAPI) {
	pi.registerTool({
		name: "ask",
		label: "Ask",
		description:
			"Ask the user a batched round of decision questions. Each question gets 2-4 mutually exclusive options (up to 9), a one-line consequence per option (what happens if it is chosen, including the cost), an optional preview block shown for the highlighted option (folder layout, config, data model), and your recommendation — marked ★ and focused, so Enter accepts it. The user picks one option and can attach a free-text note to that decision; `s` skips a question and leaves it open. Returns every decision with its note and whether it overruled your recommendation. Use for the forks only the user can settle; never for what you can measure yourself, and never for a question whose answer changes nothing.",
		promptSnippet: "Ask the user a round of decision questions, each with options, your recommendation and a note",
		promptGuidelines: [
			"Use ask when a fork belongs to the user and not to you: 2-4 mutually exclusive options, a one-line consequence each, your recommendation marked and given first, and a preview when the choice has shape. The user can attach a note to each decision — those notes are their reasoning, so carry them into whatever you write next.",
			"Use ask never for what you can measure yourself, and never for a question whose answer changes nothing: decide those, say you decided, and move on.",
		],
		parameters: AskParams,

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			return runAsk(params as { topic: string; questions: AskQuestion[] }, ctx);
		},

		renderCall(args, theme, _context) {
			const count = Array.isArray(args.questions) ? args.questions.length : 0;
			let text = theme.fg("toolTitle", theme.bold("ask "));
			text += theme.fg("muted", `${args.topic ?? ""} `);
			text += theme.fg("dim", `(${count} question${count !== 1 ? "s" : ""})`);
			return new Text(text, 0, 0);
		},

		renderResult(result, { expanded }, theme, _context) {
			const details = result.details as AskResult | undefined;
			if (!details || details.answers.length === 0) {
				const t = result.content[0];
				return new Text(theme.fg("warning", t?.type === "text" ? t.text : ""), 0, 0);
			}
			const answered = details.answers.filter((a) => !a.skipped).length;
			const overruled = details.answers.filter((a) => a.followedRecommendation === false).length;
			let text = theme.fg("success", `◆ ${answered}/${details.answers.length} decided`);
			if (overruled > 0) text += theme.fg("warning", ` · ${overruled} overruled your recommendation`);
			if (details.cancelled) text += theme.fg("warning", " (stopped early)");
			for (const a of details.answers) {
				const label = a.skipped
					? theme.fg("warning", "skipped — still open")
					: theme.fg("text", a.label ?? "");
				text += `\n  ${theme.fg("muted", `${a.title ?? a.id}:`)} ${label}`;
				if (a.note) {
					const note = expanded ? a.note : a.note.replace(/\s*\n\s*/g, " / ");
					text += `\n    ${theme.fg("dim", `note: ${note}`)}`;
				}
			}
			return new Text(text, 0, 0);
		},
	});

	pi.registerTool({
		name: "quiz",
		label: "Quiz",
		description:
			"Ask the user graded multiple-choice questions with immediate right/wrong feedback. Use during learning sessions: to probe the edge of the user's understanding (broad → specific, binary-searching each dependency strand) and to verify understanding after each teaching step. Each question carries its correct answer and an explanation; the widget grades on selection and always offers 'I don't know'. Returns per-question results (correct / incorrect / IDK).",
		promptSnippet: "Ask graded multiple-choice questions and grade them on the spot",
		promptGuidelines: [
			"Use quiz to measure what a learner already holds: give every question its correct answer and a one-to-three sentence explanation, and always add the 'I don't know' escape by leaving it out of the options.",
		],
		parameters: QuizParams,

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			return runQuiz(params as { topic: string; questions: QuizQuestion[] }, ctx);
		},

		renderCall(args, theme, _context) {
			const count = Array.isArray(args.questions) ? args.questions.length : 0;
			let text = theme.fg("toolTitle", theme.bold("quiz "));
			text += theme.fg("muted", `${args.topic ?? ""} `);
			text += theme.fg("dim", `(${count} question${count !== 1 ? "s" : ""})`);
			return new Text(text, 0, 0);
		},

		renderResult(result, _options, theme, _context) {
			const details = result.details as QuizResult | undefined;
			if (!details || details.answers.length === 0) {
				const t = result.content[0];
				return new Text(theme.fg("warning", t?.type === "text" ? t.text : ""), 0, 0);
			}
			const score = details.answers.filter((a) => a.correct).length;
			const marks = details.answers
				.map((a) => (a.correct ? theme.fg("success", "✓") : a.idk ? theme.fg("warning", "?") : theme.fg("error", "✗")))
				.join(" ");
			const status = details.cancelled ? theme.fg("warning", " (stopped early)") : "";
			return new Text(`${marks}  ${theme.fg("text", `${score}/${details.answers.length} correct`)}${status}`, 0, 0);
		},
	});
}
