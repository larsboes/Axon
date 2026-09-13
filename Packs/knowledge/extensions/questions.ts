/**
 * Quiz — graded multiple-choice questions for learning sessions.
 *
 * The model supplies questions WITH the correct answer and an explanation.
 * The widget shows one question at a time, grades immediately on selection
 * (✓/✗ + correct answer + explanation), always offers "I don't know", and
 * returns per-question results so the model can map the learner's edge.
 *
 * Used by the `teach` skill (probe phase + periodic understanding checks).
 *
 * Non-interactive modes (-p, json, rpc-without-tui): returns a graceful
 * error result instead of hanging — the model should fall back to asking
 * questions as plain text.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Key, matchesKey, Text, visibleWidth, wrapTextWithAnsi } from "@earendil-works/pi-tui";
import { Type } from "typebox";

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

export default function quiz(pi: ExtensionAPI) {
	pi.registerTool({
		name: "quiz",
		label: "Quiz",
		description:
			"Ask the user graded multiple-choice questions with immediate right/wrong feedback. Use during learning sessions: to probe the edge of the user's understanding (broad → specific, binary-searching each dependency strand) and to verify understanding after each teaching step. Each question carries its correct answer and an explanation; the widget grades on selection and always offers 'I don't know'. Returns per-question results (correct / incorrect / IDK).",
		parameters: QuizParams,

		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			if (ctx.mode !== "tui") {
				return {
					content: [
						{
							type: "text" as const,
							text: "Error: quiz UI unavailable (non-interactive mode). Ask the questions as plain numbered text instead.",
						},
					],
					details: { topic: params.topic, answers: [], cancelled: true } satisfies QuizResult,
				};
			}
			const questions = params.questions as QuizQuestion[];
			if (questions.length === 0) {
				return {
					content: [{ type: "text" as const, text: "Error: no questions provided" }],
					details: { topic: params.topic, answers: [], cancelled: true } satisfies QuizResult,
				};
			}
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

					function addPrefixed(prefix: string, text: string) {
						const pw = visibleWidth(prefix);
						const wrapped = wrapTextWithAnsi(text, Math.max(1, w - pw));
						const cont = " ".repeat(pw);
						for (let i = 0; i < wrapped.length; i++) {
							lines.push(`${i === 0 ? prefix : cont}${wrapped[i]}`);
						}
					}

					lines.push(theme.fg("accent", "─".repeat(w)));
					const score = answers.filter((a) => a.correct).length;
					addPrefixed(
						" ",
						theme.fg("muted", `${params.topic} · question ${qIndex + 1}/${questions.length} · ${score} correct`),
					);
					lines.push("");
					addPrefixed(" ", theme.fg("text", theme.bold(q.prompt)));
					lines.push("");

					for (let i = 0; i < opts.length; i++) {
						const isIdk = i === q.options.length;
						const selected = i === optionIndex;
						let prefix = selected && !revealed ? theme.fg("accent", "> ") : "  ";
						let color: string = selected && !revealed ? "accent" : isIdk ? "muted" : "text";
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
						addPrefixed(prefix, theme.fg(color as Parameters<typeof theme.fg>[0], `${i + 1}. ${opts[i]}${suffix}`));
					}

					lines.push("");
					if (revealed && answer) {
						const verdict = answer.correct
							? theme.fg("success", "✓ Correct")
							: answer.idk
								? theme.fg("warning", `— The answer: ${q.options[q.correctIndex]}`)
								: theme.fg("error", `✗ Incorrect — correct: ${q.options[q.correctIndex]}`);
						addPrefixed(" ", verdict);
						if (q.explanation) {
							lines.push("");
							addPrefixed(" ", theme.fg("muted", q.explanation));
						}
						lines.push("");
						addPrefixed(" ", theme.fg("dim", "Enter/Space for next • Esc to stop"));
					} else {
						addPrefixed(" ", theme.fg("dim", "↑↓ or 1-9 • Enter to answer • Esc to stop"));
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
			const marks = details.answers.map((a) => (a.correct ? theme.fg("success", "✓") : a.idk ? theme.fg("warning", "?") : theme.fg("error", "✗"))).join(" ");
			const status = details.cancelled ? theme.fg("warning", " (stopped early)") : "";
			return new Text(`${marks}  ${theme.fg("text", `${score}/${details.answers.length} correct`)}${status}`, 0, 0);
		},
	});
}
