# The investigation interview

Welcome the user to an all-workflows briefing. Show how many conversations were captured, how many contain findings, and the most common finding types by affected conversation count. Ask **What would you like to investigate?** Offer two or three questions grounded in that briefing. A user can type a different question at any point.

The sidebar lists recorded workflows. Each has a homepage with the same briefing and composer scoped to its conversations. The workspace homepage can investigate across scopes. A user can also name a workflow in the composer.

## How the interview works

| User intent | What appears in the conversation | Evidence it uses |
| --- | --- | --- |
| What needs attention? | A short briefing and a few questions to pursue | Scoped counts and recurring themes |
| What happened here? | An example and the inputs or results relevant to the question | Recorded turns, source locations, and tool observations |
| Did this happen elsewhere? | A count, a trend when dates are available, and representative conversations | Observations grouped by matching evidence |
| Did a change help? | A comparison and a short explanation of the measured outcome | Original and edited trials, including incomplete evidence |
| What should I try next? | One suggested check or experiment | The verified limits of the evidence already shown |

The composer stays available through the investigation. Follow-up questions can refer to the previous example. A question about another workflow should change scope in the conversation. Shared links can carry an exact scope and evidence location. The main interview needs no filter form or timeline navigation. Exact evidence pages remain available when a user wants to check a claim.

## Evidence and presentation

The Rust sidecar keeps counts and evidence references authoritative. Without a model, the browser answers a bounded set of question types using those records. A configured model can interpret open-ended questions and choose evidence and views. It works with a bounded catalog of references and cannot supply arbitrary links or overwrite measured values. The A2UI Lit renderer receives the components for each answer. Inferences about source influence need measured evidence under the tested conditions. [A2UI messages](https://a2ui.org/reference/messages/) · [A2UI Basic Catalog](https://a2ui.org/concepts/catalogs/)

System appearance is the default. A light or dark selection persists for the workspace. The interview and linked evidence need readable text, clear focus, and usable layouts on phones and desktops.

## Delivery and checks

1. Ship the briefing, question composer, scoped evidence replies, and theme controls. Keep bookmarked evidence and routes working during the change.
2. Broaden question routing and generated views, including questions about unfamiliar workflows, changes of scope, comparisons across findings, and incomplete evidence. Check a local model and the saved-evidence path against the same questions.
3. Update and record the repository demo. Begin with the briefing, type the first question, and follow the same issue through the source comparison, measured outcome, and recurring cases.
4. Test the flow with the exchange fixture and the full public workload. Verify counts against the API, exact links, time to first answer, keyboard operation, contrast, small screens, unsupported questions, and provider failures. Review where the ranking or answer chose the wrong evidence and rerun those cases after each change.

The first pass uses the exchange demo and a separate copy of 18,648 recorded Exgentic requests across six workflows. The browser opens a representative finding by typing a question and checks the exact evidence on desktop and mobile. The exchange recording follows the same interview with saved evidence. The next evaluation needs questions outside the known discovery, source, recurrence, and action views, including comparisons between workflows and questions that require inspecting tool activity with no existing finding.
