import type { GeneratedDoc } from "./types";

// Seeded PRNG for reproducibility
class Rng {
  private state: number;
  constructor(seed: number) {
    this.state = seed;
  }
  next(): number {
    this.state = (this.state * 1664525 + 1013904223) & 0x7fffffff;
    return this.state / 0x7fffffff;
  }
  pick<T>(arr: readonly T[]): T {
    return arr[Math.floor(this.next() * arr.length)];
  }
  pickN<T>(arr: readonly T[], n: number): T[] {
    const copy = [...arr];
    const result: T[] = [];
    for (let i = 0; i < Math.min(n, copy.length); i++) {
      const idx = Math.floor(this.next() * copy.length);
      result.push(copy.splice(idx, 1)[0]);
    }
    return result;
  }
  int(min: number, max: number): number {
    return min + Math.floor(this.next() * (max - min + 1));
  }
  chance(p: number): boolean {
    return this.next() < p;
  }
  shuffle<T>(arr: T[]): T[] {
    const copy = [...arr];
    for (let i = copy.length - 1; i > 0; i--) {
      const j = Math.floor(this.next() * (i + 1));
      [copy[i], copy[j]] = [copy[j], copy[i]];
    }
    return copy;
  }
}

// --- Name pools ---

const COMPANY_NAMES = [
  "Meridian Capital Partners",
  "Atlas Infrastructure Holdings",
  "Pinnacle Technology Solutions",
  "Vanguard Biomedical Research",
  "Ironclad Manufacturing Corp",
  "Sentinel Defense Systems",
  "Horizon Renewable Energy",
  "Sterling Financial Group",
  "Apex Logistics International",
  "Nexus Pharmaceutical Inc",
  "Crossroads Real Estate Development",
  "Titan Aerospace Industries",
  "Cascade Water Utilities",
  "Broadleaf Agricultural Corp",
  "Quantum Data Analytics",
  "Blackstone Ridge Mining",
  "Pacific Maritime Shipping",
  "Cornerstone Construction Holdings",
  "Redwood Healthcare Systems",
  "Silverline Communications",
  "Cobalt Energy Partners",
  "Westfield Retail Group",
  "Northstar Aviation Ltd",
  "Oakmont Wealth Management",
  "Granite State Insurance Co",
] as const;

const PERSON_NAMES = [
  "James Whitfield",
  "Sarah Chen",
  "Michael Rodriguez",
  "Elizabeth Warren-Hughes",
  "David Nakamura",
  "Patricia O'Connell",
  "Robert Krishnamurthy",
  "Jennifer Blackwell",
  "Thomas Andersson",
  "Maria Gonzalez-Reyes",
  "William Chang",
  "Catherine Doyle",
  "Richard Petrov",
  "Angela Morrison",
  "Christopher Okafor",
] as const;

const LAW_FIRMS = [
  "Sullivan & Cromwell LLP",
  "Davis Polk & Wardwell",
  "Wachtell Lipton Rosen & Katz",
  "Cravath Swaine & Moore",
  "Kirkland & Ellis LLP",
  "Skadden Arps Slate Meagher & Flom",
  "Latham & Watkins LLP",
  "Simpson Thacher & Bartlett",
  "Cleary Gottlieb Steen & Hamilton",
  "Paul Weiss Rifkind Wharton & Garrison",
] as const;

const JURISDICTIONS = [
  "Delaware",
  "New York",
  "California",
  "Texas",
  "Illinois",
  "Massachusetts",
  "Florida",
  "Pennsylvania",
  "Virginia",
  "District of Columbia",
  "Nevada",
  "Georgia",
] as const;

const COURTS = [
  { name: "Delaware Court of Chancery", abbr: "Del. Ch." },
  { name: "Supreme Court of Delaware", abbr: "Del." },
  { name: "Southern District of New York", abbr: "S.D.N.Y." },
  { name: "Eastern District of Texas", abbr: "E.D. Tex." },
  { name: "Northern District of California", abbr: "N.D. Cal." },
  { name: "Central District of California", abbr: "C.D. Cal." },
  { name: "District of Delaware", abbr: "D. Del." },
  { name: "Southern District of Texas", abbr: "S.D. Tex." },
  { name: "Northern District of Illinois", abbr: "N.D. Ill." },
  { name: "Eastern District of Virginia", abbr: "E.D. Va." },
  { name: "District of Massachusetts", abbr: "D. Mass." },
  { name: "Third Circuit Court of Appeals", abbr: "3d Cir." },
  { name: "Second Circuit Court of Appeals", abbr: "2d Cir." },
  { name: "Ninth Circuit Court of Appeals", abbr: "9th Cir." },
  { name: "Fifth Circuit Court of Appeals", abbr: "5th Cir." },
] as const;

const CONTRACT_TYPES = [
  "Mutual Non-Disclosure Agreement",
  "Master Services Agreement",
  "Stock Purchase Agreement",
  "Asset Purchase Agreement",
  "Software License Agreement",
  "Employment Agreement",
  "Consulting Services Agreement",
  "Joint Venture Agreement",
  "Supply Agreement",
  "Distribution Agreement",
  "Franchise Agreement",
  "Commercial Lease Agreement",
  "Merger Agreement",
  "Subscription Agreement",
  "Credit Agreement",
  "Loan Agreement",
  "Guaranty Agreement",
  "Indemnification Agreement",
  "Settlement Agreement",
  "Technology Transfer Agreement",
] as const;

const LEGAL_TOPICS = [
  "breach of fiduciary duty",
  "securities fraud",
  "patent infringement",
  "trade secret misappropriation",
  "antitrust violation",
  "breach of contract",
  "tortious interference",
  "unjust enrichment",
  "fraudulent conveyance",
  "shareholder derivative action",
  "class action certification",
  "ERISA compliance",
  "environmental remediation",
  "FCPA enforcement",
  "data privacy violation",
  "GDPR non-compliance",
  "employment discrimination",
  "wrongful termination",
  "merger objection",
  "appraisal rights",
  "books and records inspection",
  "demand futility",
  "entire fairness review",
  "business judgment rule",
  "Revlon duties",
] as const;

const STATUTES = [
  { cite: "15 U.S.C. § 78j(b)", name: "Securities Exchange Act Section 10(b)" },
  { cite: "17 C.F.R. § 240.10b-5", name: "Rule 10b-5" },
  { cite: "8 Del. C. § 220", name: "Delaware Books and Records" },
  { cite: "8 Del. C. § 262", name: "Delaware Appraisal Statute" },
  { cite: "8 Del. C. § 102(b)(7)", name: "Delaware Exculpation Statute" },
  { cite: "15 U.S.C. § 1", name: "Sherman Antitrust Act Section 1" },
  { cite: "35 U.S.C. § 271", name: "Patent Infringement" },
  { cite: "18 U.S.C. § 1836", name: "Defend Trade Secrets Act" },
  { cite: "29 U.S.C. § 1132", name: "ERISA Civil Enforcement" },
  { cite: "42 U.S.C. § 2000e", name: "Title VII Civil Rights" },
  { cite: "28 U.S.C. § 1332", name: "Diversity Jurisdiction" },
  { cite: "Fed. R. Civ. P. 12(b)(6)", name: "Failure to State a Claim" },
  { cite: "Fed. R. Civ. P. 23", name: "Class Actions" },
  { cite: "Fed. R. Civ. P. 56", name: "Summary Judgment" },
  { cite: "N.Y. Bus. Corp. Law § 626", name: "NY Shareholder Derivative Actions" },
  { cite: "Cal. Corp. Code § 1800", name: "California Involuntary Dissolution" },
  { cite: "UCC § 2-207", name: "Battle of the Forms" },
  { cite: "Restatement (Second) of Contracts § 90", name: "Promissory Estoppel" },
] as const;

const CASE_LAW = [
  { name: "Revlon, Inc. v. MacAndrews & Forbes Holdings", cite: "506 A.2d 173 (Del. 1986)" },
  { name: "Smith v. Van Gorkom", cite: "488 A.2d 858 (Del. 1985)" },
  { name: "Weinberger v. UOP, Inc.", cite: "457 A.2d 701 (Del. 1983)" },
  { name: "Unocal Corp. v. Mesa Petroleum Co.", cite: "493 A.2d 946 (Del. 1985)" },
  { name: "Aronson v. Lewis", cite: "473 A.2d 805 (Del. 1984)" },
  { name: "Kahn v. M & F Worldwide Corp.", cite: "88 A.3d 635 (Del. 2014)" },
  { name: "Corwin v. KKR Financial Holdings", cite: "125 A.3d 304 (Del. 2015)" },
  { name: "In re Trulia, Inc. Stockholder Litigation", cite: "129 A.3d 884 (Del. Ch. 2016)" },
  { name: "Blasius Industries, Inc. v. Atlas Corp.", cite: "564 A.2d 651 (Del. Ch. 1988)" },
  { name: "Zapata Corp. v. Maldonado", cite: "430 A.2d 779 (Del. 1981)" },
  { name: "In re Caremark International Inc. Derivative Litigation", cite: "698 A.2d 959 (Del. Ch. 1996)" },
  { name: "Stone v. Ritter", cite: "911 A.2d 362 (Del. 2006)" },
  { name: "Marchand v. Barnhill", cite: "212 A.3d 805 (Del. 2019)" },
  { name: "International Shoe Co. v. Washington", cite: "326 U.S. 310 (1945)" },
  { name: "Erie Railroad Co. v. Tompkins", cite: "304 U.S. 64 (1938)" },
  { name: "Celotex Corp. v. Catrett", cite: "477 U.S. 317 (1986)" },
  { name: "Twombly v. Bell Atlantic Corp.", cite: "550 U.S. 544 (2007)" },
  { name: "Ashcroft v. Iqbal", cite: "556 U.S. 662 (2009)" },
  { name: "Alice Corp. v. CLS Bank International", cite: "573 U.S. 208 (2014)" },
  { name: "eBay Inc. v. MercExchange, L.L.C.", cite: "547 U.S. 388 (2006)" },
] as const;

// --- Content building blocks ---

const CONTRACT_RECITALS = [
  "WHEREAS, the Company is engaged in the business of developing and commercializing proprietary technology solutions for enterprise customers;",
  "WHEREAS, the Parties desire to enter into this Agreement to set forth the terms and conditions governing their business relationship;",
  "WHEREAS, the Receiving Party acknowledges that in the course of the business relationship, it may receive Confidential Information of substantial value;",
  "WHEREAS, the Company has developed certain intellectual property that it desires to license to the Licensee under the terms set forth herein;",
  "WHEREAS, the Seller desires to sell, and the Buyer desires to purchase, substantially all of the assets of the Business, subject to the terms and conditions of this Agreement;",
  "WHEREAS, the Parties have been engaged in arms-length negotiations concerning the proposed transaction described herein;",
  "WHEREAS, the Board of Directors of the Company has determined that the Transaction is advisable and in the best interests of the Company and its stockholders;",
];

const CONTRACT_CLAUSES = {
  indemnification: `INDEMNIFICATION. Each Party (as "Indemnifying Party") shall indemnify, defend, and hold harmless the other Party and its officers, directors, employees, agents, successors, and assigns (collectively, "Indemnified Parties") from and against any and all losses, damages, liabilities, deficiencies, claims, actions, judgments, settlements, interest, awards, penalties, fines, costs, or expenses of whatever kind, including reasonable attorneys' fees, that are incurred by the Indemnified Parties arising out of or related to: (a) any breach of any representation or warranty made by the Indemnifying Party; (b) any breach or non-fulfillment of any covenant or obligation of the Indemnifying Party; or (c) any negligent or more culpable act or omission of the Indemnifying Party.`,

  limitation_of_liability: `LIMITATION OF LIABILITY. IN NO EVENT SHALL EITHER PARTY BE LIABLE TO THE OTHER PARTY FOR ANY CONSEQUENTIAL, INCIDENTAL, INDIRECT, EXEMPLARY, SPECIAL, OR PUNITIVE DAMAGES, INCLUDING ANY DAMAGES FOR BUSINESS INTERRUPTION, LOSS OF USE, DATA, REVENUE, OR PROFIT, WHETHER ARISING OUT OF BREACH OF CONTRACT, TORT (INCLUDING NEGLIGENCE), OR OTHERWISE, REGARDLESS OF WHETHER SUCH DAMAGES WERE FORESEEABLE AND WHETHER OR NOT SUCH PARTY WAS ADVISED OF THE POSSIBILITY OF SUCH DAMAGES. NOTWITHSTANDING THE FOREGOING, NOTHING IN THIS SECTION SHALL LIMIT A PARTY'S LIABILITY FOR: (i) FRAUD OR WILLFUL MISCONDUCT; (ii) BREACH OF CONFIDENTIALITY OBLIGATIONS; OR (iii) INDEMNIFICATION OBLIGATIONS UNDER THIS AGREEMENT. THE AGGREGATE LIABILITY OF EACH PARTY UNDER THIS AGREEMENT SHALL NOT EXCEED THE TOTAL AMOUNT OF FEES PAID OR PAYABLE UNDER THIS AGREEMENT DURING THE TWELVE (12) MONTH PERIOD PRECEDING THE DATE ON WHICH THE CLAIM AROSE.`,

  termination: `TERMINATION. This Agreement may be terminated: (a) by mutual written agreement of the Parties; (b) by either Party upon thirty (30) days' prior written notice if the other Party materially breaches any provision of this Agreement and fails to cure such breach within the thirty-day notice period; (c) by either Party immediately upon written notice if the other Party becomes insolvent, files a petition in bankruptcy, or has a receiver appointed for a substantial portion of its assets; or (d) by either Party upon ninety (90) days' prior written notice without cause. Upon termination or expiration of this Agreement, each Party shall promptly return or destroy all Confidential Information of the other Party in its possession and shall certify in writing that it has complied with this obligation.`,

  governing_law: `GOVERNING LAW AND DISPUTE RESOLUTION. This Agreement shall be governed by and construed in accordance with the laws of the State of {{jurisdiction}}, without giving effect to any choice or conflict of law provision or rule. Any legal suit, action, or proceeding arising out of or related to this Agreement shall be instituted exclusively in the courts of {{jurisdiction}}, and each Party irrevocably submits to the exclusive jurisdiction of such courts. The Parties agree that before initiating any litigation, they shall first attempt to resolve the dispute through good-faith negotiation for a period of thirty (30) days, followed by mediation administered by JAMS under its then-applicable rules.`,

  representations: `REPRESENTATIONS AND WARRANTIES. Each Party represents and warrants to the other Party that: (a) it is duly organized, validly existing, and in good standing under the laws of its state of incorporation; (b) it has full corporate power and authority to enter into this Agreement and perform its obligations hereunder; (c) the execution, delivery, and performance of this Agreement have been duly authorized by all necessary corporate action; (d) this Agreement constitutes the valid and binding obligation of such Party, enforceable against it in accordance with its terms, subject to applicable bankruptcy, insolvency, and similar laws affecting creditors' rights generally; and (e) the execution and performance of this Agreement will not violate or conflict with any agreement to which such Party is bound.`,

  confidentiality: `CONFIDENTIALITY. "Confidential Information" means any non-public information disclosed by one Party to the other, whether orally, in writing, or by inspection, including but not limited to: trade secrets, business plans, financial data, customer lists, technical specifications, algorithms, source code, and any other proprietary information. The Receiving Party shall: (a) use the Confidential Information solely for the purpose of performing its obligations under this Agreement; (b) protect the Confidential Information using at least the same degree of care it uses to protect its own confidential information, but in no event less than reasonable care; (c) not disclose the Confidential Information to any third party without the prior written consent of the Disclosing Party, except to those employees, agents, and contractors who have a need to know and are bound by obligations of confidentiality no less restrictive than those set forth herein. The obligations of confidentiality shall survive termination of this Agreement for a period of five (5) years.`,

  ip_assignment: `INTELLECTUAL PROPERTY ASSIGNMENT. All Work Product created by the Service Provider in the course of performing Services under this Agreement shall be considered "work made for hire" as defined under the Copyright Act of 1976. To the extent that any Work Product does not qualify as work made for hire, the Service Provider hereby irrevocably assigns to the Company all right, title, and interest in and to such Work Product, including all intellectual property rights therein. The Service Provider agrees to execute any documents and take any actions reasonably necessary to effectuate such assignment. The Service Provider retains no rights in the Work Product except as expressly granted herein.`,

  force_majeure: `FORCE MAJEURE. Neither Party shall be liable for any failure or delay in performing its obligations under this Agreement to the extent that such failure or delay results from circumstances beyond the reasonable control of that Party, including but not limited to: acts of God, natural disasters, war, terrorism, riots, embargoes, acts of civil or military authorities, fire, floods, epidemics, pandemics, quarantine restrictions, strikes, labor disputes, or shortages of transportation, facilities, fuel, energy, labor, or materials. The affected Party shall provide prompt written notice to the other Party of the force majeure event and shall use commercially reasonable efforts to mitigate its effects.`,

  non_compete: `NON-COMPETITION AND NON-SOLICITATION. During the Term and for a period of twenty-four (24) months following the termination of this Agreement, the Restricted Party shall not, directly or indirectly: (a) engage in any business that competes with the Business within the Territory; (b) solicit, recruit, or hire any employee or contractor of the Company; or (c) solicit or attempt to divert any customer, client, or business relationship of the Company. The Restricted Party acknowledges that the restrictions in this Section are reasonable in scope and duration and are necessary to protect the Company's legitimate business interests, including its goodwill, trade secrets, and customer relationships.`,

  insurance: `INSURANCE. During the Term, each Party shall maintain at its own expense: (a) commercial general liability insurance with limits of not less than $2,000,000 per occurrence and $5,000,000 in the aggregate; (b) professional liability / errors and omissions insurance with limits of not less than $3,000,000 per claim; (c) workers' compensation insurance as required by applicable law; and (d) cyber liability insurance with limits of not less than $5,000,000 per occurrence, covering data breaches, network security failures, and privacy violations. Upon request, each Party shall provide the other Party with certificates of insurance evidencing such coverage.`,
};

const FILING_SECTIONS = {
  statement_of_facts: (rng: Rng, plaintiff: string, defendant: string, topic: string) => {
    const date = `${rng.pick(["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"])} ${rng.int(1, 28)}, ${rng.int(2019, 2025)}`;
    return `STATEMENT OF FACTS

${plaintiff} is a ${rng.pick(["Delaware corporation", "New York limited liability company", "California corporation", "publicly traded company incorporated in Delaware"])} with its principal place of business in ${rng.pick(JURISDICTIONS)}. ${defendant} is a ${rng.pick(["Delaware limited partnership", "corporation organized under the laws of Nevada", "limited liability company organized under the laws of Texas", "publicly traded corporation incorporated in Delaware"])} with its principal offices located in ${rng.pick(JURISDICTIONS)}.

On or about ${date}, the parties entered into a ${rng.pick(["Master Services Agreement", "Stock Purchase Agreement", "Merger Agreement", "Asset Purchase Agreement", "Joint Venture Agreement"])} (the "Agreement") pursuant to which ${defendant} agreed to ${rng.pick(["provide certain technology development services", "acquire substantially all of the assets of " + plaintiff, "merge with a wholly-owned subsidiary of " + plaintiff, "license its proprietary software platform", "invest $" + rng.int(5, 500) + " million in exchange for a " + rng.int(15, 49) + "% equity stake"])}.

Under the terms of the Agreement, ${defendant} was required to ${rng.pick(["deliver a fully functional software platform by the milestone date", "make a series of installment payments totaling approximately $" + rng.int(10, 200) + " million", "obtain all necessary regulatory approvals within the specified timeframe", "maintain certain financial covenants throughout the term", "complete due diligence and provide accurate representations and warranties"])}. Despite repeated assurances and written commitments, ${defendant} failed to satisfy these obligations.

Specifically, beginning on or about ${rng.pick(["three months", "six months", "nine months", "one year"])} after the execution of the Agreement, ${defendant} ${rng.pick(["ceased making required payments without justification", "began diverting key personnel and resources away from the project", "failed to meet critical development milestones", "breached multiple material representations contained in the Agreement", "engaged in a pattern of conduct designed to extract value from " + plaintiff + " while avoiding its contractual commitments"])}. These actions constitute a material breach of the Agreement and have caused ${plaintiff} to suffer damages in excess of $${rng.int(5, 300)} million.`;
  },

  legal_argument: (rng: Rng, topic: string) => {
    const cases = rng.pickN(CASE_LAW, 3);
    const statutes = rng.pickN(STATUTES, 2);
    return `ARGUMENT

I. ${topic.toUpperCase()}

A. Legal Standard

The legal standard governing claims for ${topic} is well established in this jurisdiction. Under ${statutes[0].name}, ${statutes[0].cite}, a plaintiff must demonstrate: (1) the existence of a duty or obligation; (2) a breach of that duty; (3) a causal connection between the breach and the alleged harm; and (4) actual damages resulting therefrom. See ${cases[0].name}, ${cases[0].cite}.

The ${rng.pick(["Supreme Court", "Court of Appeals", "this Court"])} has repeatedly emphasized that the standard requires more than conclusory allegations. As the Court explained in ${cases[1].name}, ${cases[1].cite}, the plaintiff must plead "enough facts to state a claim to relief that is plausible on its face." This standard demands that the complaint contain sufficient factual matter, accepted as true, to allow the court to draw a reasonable inference of liability.

B. Application to the Facts

Here, the evidence overwhelmingly supports ${rng.pick(["Plaintiff's", "Defendant's", "Movant's"])} position. The undisputed record demonstrates that ${rng.pick(["the defendant had actual knowledge of the material facts at issue", "the representations made during the transaction were false and misleading", "the board of directors failed to satisfy its fiduciary obligations", "the challenged conduct caused direct and foreseeable harm to the plaintiffs", "the contractual provisions at issue are clear and unambiguous"])}.

In ${cases[2].name}, ${cases[2].cite}, the Court addressed a substantially similar factual scenario and held that such conduct ${rng.pick(["constitutes a breach of the duty of loyalty", "satisfies the elements of a fraud claim", "warrants the imposition of equitable relief", "gives rise to liability under the applicable statutory framework"])}. The reasoning in that case applies with equal force here.

Moreover, under ${statutes[1].name}, ${statutes[1].cite}, the ${rng.pick(["statutory framework provides an independent basis for relief", "regulatory requirements impose affirmative obligations that the defendant failed to satisfy", "legislative history confirms that Congress intended to provide broad protection against exactly this type of conduct"])}.`;
  },

  prayer_for_relief: (rng: Rng, plaintiff: string) => {
    return `PRAYER FOR RELIEF

WHEREFORE, ${plaintiff} respectfully requests that this Court:

(a) Enter judgment in favor of ${plaintiff} and against Defendant on all counts of the Complaint;

(b) Award ${plaintiff} compensatory damages in an amount to be proven at trial, but in no event less than $${rng.int(5, 500)} million;

(c) ${rng.pick([
      "Award punitive damages in an amount sufficient to deter future misconduct;",
      "Enter a preliminary and permanent injunction prohibiting Defendant from continuing the challenged conduct;",
      "Order specific performance of the Agreement;",
      "Impose a constructive trust on the assets wrongfully obtained by Defendant;",
      "Declare the rights and obligations of the parties under the Agreement;",
    ])}

(d) Award ${plaintiff} its reasonable attorneys' fees, costs, and expenses incurred in this action;

(e) Award pre-judgment and post-judgment interest at the maximum rate permitted by law; and

(f) Grant such other and further relief as this Court deems just and proper.`;
  },
};

const MEMO_TEMPLATES = {
  case_analysis: (rng: Rng) => {
    const topic = rng.pick(LEGAL_TOPICS);
    const cases = rng.pickN(CASE_LAW, 4);
    const client = rng.pick(COMPANY_NAMES);
    const opponent = rng.pick(COMPANY_NAMES.filter((c) => c !== client));
    const attorney = rng.pick(PERSON_NAMES);
    const partner = rng.pick(PERSON_NAMES.filter((p) => p !== attorney));

    return {
      title: `Legal Memorandum re: ${topic} — ${client} v. ${opponent}`,
      body: `PRIVILEGED AND CONFIDENTIAL
ATTORNEY WORK PRODUCT

MEMORANDUM

TO:      ${partner}, Partner
FROM:    ${attorney}, Associate
DATE:    ${rng.pick(["January", "February", "March", "April", "May", "June"])} ${rng.int(1, 28)}, ${rng.int(2024, 2026)}
RE:      Analysis of ${topic} Claims — ${client} v. ${opponent}

I. QUESTION PRESENTED

Whether ${client} has viable claims for ${topic} against ${opponent} based on ${opponent}'s ${rng.pick(["failure to disclose material information during the transaction", "conduct in connection with the negotiation and execution of the Agreement", "post-closing breach of the representations and warranties", "interference with " + client + "'s business relationships and contractual rights", "unauthorized use and disclosure of " + client + "'s confidential information and trade secrets"])}.

II. SHORT ANSWER

${rng.pick(["Yes, likely.", "Probably yes, with caveats.", "The claim is viable but faces significant hurdles.", "The analysis is fact-dependent, but the balance of factors favors our client."])} Based on the available evidence and controlling precedent, ${client} ${rng.pick(["has strong", "has moderately strong", "has colorable but contested"])} claims for ${topic}. The key factors supporting this conclusion are: (1) ${rng.pick(["the clear documentary evidence of " + opponent + "'s knowledge", "the temporal proximity between the challenged conduct and the resulting harm", "the well-established precedent supporting liability under these circumstances", "the strength of the contractual provisions at issue"])}; and (2) ${rng.pick(["the substantial damages that can be quantified through expert testimony", "the availability of equitable remedies that may provide meaningful relief", "the favorable procedural posture of the case", "the weakness of the likely defenses"])}.

III. DISCUSSION

A. Applicable Legal Framework

The doctrine of ${topic} has been extensively developed in this jurisdiction. The seminal case is ${cases[0].name}, ${cases[0].cite}, where the Court established the foundational framework for analyzing such claims. Under the ${cases[0].name} framework, a plaintiff must establish the following elements:

First, the plaintiff must demonstrate ${rng.pick(["the existence of a fiduciary relationship or other special duty owed by the defendant", "that the defendant made a material misrepresentation or omission", "that the defendant's conduct constituted a breach of an enforceable contractual obligation", "that the defendant engaged in conduct that was wrongful and caused harm to the plaintiff"])}. ${cases[1].name}, ${cases[1].cite}.

Second, the plaintiff must show ${rng.pick(["that the defendant acted with scienter or, at minimum, with reckless disregard for the truth", "that the breach was material and not merely technical or trivial", "that the plaintiff reasonably relied on the defendant's representations to its detriment", "proximate causation between the defendant's wrongful conduct and the plaintiff's injuries"])}. Id.

Third, the plaintiff must prove ${rng.pick(["actual damages flowing from the defendant's misconduct", "that the defendant was unjustly enriched at the plaintiff's expense", "that equitable relief is necessary to prevent irreparable harm", "that the challenged conduct had a material adverse effect on the plaintiff's business"])}. ${cases[2].name}, ${cases[2].cite}.

B. Application to Our Facts

The facts here present a ${rng.pick(["compelling", "reasonably strong", "complex but ultimately favorable"])} case for ${topic}. The key evidence supporting our position includes:

The documentary record contains ${rng.pick(["extensive email correspondence demonstrating that " + opponent + "'s senior management was aware of the material issues and chose to conceal them", "internal memoranda from " + opponent + " showing that its board of directors approved the challenged transaction despite knowing that the representations were inaccurate", "financial records establishing that " + opponent + " systematically diverted assets and opportunities that belonged to " + client, "contemporaneous communications contradicting " + opponent + "'s stated justifications for its conduct"])}.

The testimony of ${rng.pick(PERSON_NAMES)} is particularly significant because it ${rng.pick(["directly contradicts " + opponent + "'s public statements about the transaction", "establishes the timeline of events and demonstrates " + opponent + "'s knowledge of the key facts", "provides firsthand evidence of the decision-making process within " + opponent + "'s management team", "corroborates the documentary evidence and fills gaps in the written record"])}.

C. Potential Defenses and Counterarguments

${opponent} will likely raise several defenses, including: (1) ${rng.pick(["the business judgment rule", "the statute of limitations", "the economic loss doctrine", "waiver and estoppel", "failure to mitigate damages"])}; (2) ${rng.pick(["the exculpation clause in the certificate of incorporation", "the no-reliance provision in the Agreement", "the voluntary dismissal of the prior action", "the safe harbor provisions of the applicable statute", "the sophisticated party defense"])}; and (3) ${rng.pick(["lack of standing", "failure to make a pre-suit demand", "the de minimis nature of the alleged harm", "independent intervening causes", "contributory fault of the plaintiff"])}.

These defenses have varying degrees of merit. The ${rng.pick(["business judgment rule defense is the most significant concern", "statute of limitations argument can likely be overcome through the discovery rule", "contractual defenses are weakened by the evidence of fraud", "most serious vulnerability is the damages quantification"])}, but on balance, the strengths of our position outweigh these risks.

IV. CONCLUSION AND RECOMMENDATION

Based on the foregoing analysis, I recommend that ${client} ${rng.pick(["proceed with filing the complaint and pursue expedited discovery", "initiate settlement discussions while preparing for litigation", "file a motion for preliminary injunctive relief before the scheduled closing date", "send a formal demand letter outlining the claims and proposed resolution", "engage an expert witness to quantify the damages before making a litigation decision"])}. The estimated litigation budget for this matter through trial is $${rng.int(1, 15)} million, with a projected timeline of ${rng.int(12, 36)} months.

${EXTRA_MEMO_DISCUSSION[0](rng, client, opponent, topic)}`,
    };
  },

  regulatory_analysis: (rng: Rng) => {
    const regulation = rng.pick([
      "GDPR Article 28 data processing requirements",
      "SEC Regulation S-K disclosure obligations",
      "CFPB unfair, deceptive, or abusive acts or practices",
      "OFAC sanctions compliance obligations",
      "FDA 21 C.F.R. Part 820 quality system regulations",
      "SOX Section 404 internal controls over financial reporting",
      "HIPAA Privacy Rule covered entity obligations",
      "CCPA consumer data rights and opt-out mechanisms",
      "AML/BSA customer due diligence requirements",
      "EPA Clean Air Act emission standards",
    ]);
    const client = rng.pick(COMPANY_NAMES);
    const attorney = rng.pick(PERSON_NAMES);

    return {
      title: `Regulatory Compliance Memorandum — ${regulation}`,
      body: `MEMORANDUM

TO:      General Counsel, ${client}
FROM:    ${attorney}
DATE:    ${rng.pick(["March", "April", "May", "June", "July"])} ${rng.int(1, 28)}, ${rng.int(2024, 2026)}
RE:      Compliance Assessment — ${regulation}

I. EXECUTIVE SUMMARY

This memorandum summarizes our assessment of ${client}'s compliance with ${regulation}. Based on our review of the company's current policies, procedures, and operational practices, we have identified ${rng.int(3, 12)} areas requiring remediation, ${rng.int(1, 5)} of which present significant regulatory risk.

The most critical findings relate to ${rng.pick(["inadequate documentation of the company's data processing activities and legal bases for processing", "gaps in the company's internal reporting mechanisms and escalation procedures", "deficiencies in the company's vendor management and third-party oversight program", "the absence of formal risk assessment procedures required by the applicable regulatory framework", "insufficient training and awareness programs for employees handling regulated activities"])}.

II. REGULATORY FRAMEWORK

${regulation} imposes the following key obligations on entities in ${client}'s position:

1. ${rng.pick(["Implementation of appropriate technical and organizational measures to ensure compliance", "Maintenance of complete and accurate records of all regulated activities", "Designation of a qualified compliance officer with sufficient authority and resources", "Establishment of internal controls designed to prevent, detect, and remediate violations", "Periodic reporting to the relevant regulatory authority on the entity's compliance status"])}

2. ${rng.pick(["Conducting regular risk assessments and implementing appropriate mitigation measures", "Providing clear and transparent disclosures to affected parties", "Implementing robust incident response and breach notification procedures", "Ensuring that all third-party service providers are contractually bound to equivalent standards", "Maintaining adequate capital reserves and financial safeguards"])}

3. ${rng.pick(["Annual training for all personnel involved in regulated activities", "Independent auditing of compliance programs on at least an annual basis", "Prompt remediation of identified deficiencies and documentation of corrective actions", "Cooperation with regulatory investigations and examinations", "Whistleblower protection and anonymous reporting mechanisms"])}

III. KEY FINDINGS

Finding 1 (HIGH RISK): ${rng.pick(["The company's current data processing agreements with third-party vendors do not contain all required contractual provisions", "The company has not conducted the required periodic risk assessments within the prescribed timeframe", "The company's incident response plan has not been tested or updated in over 18 months", "Several critical compliance controls were found to be operating ineffectively during the review period", "The company's training records indicate that approximately 30% of relevant personnel have not completed required compliance training"])}

Finding 2 (MEDIUM RISK): ${rng.pick(["The company's documentation retention practices do not fully align with regulatory requirements", "Internal audit identified gaps in the monitoring of automated decision-making systems", "The company's policies reference superseded regulatory guidance and need to be updated", "Cross-border data transfers lack appropriate safeguards under the current regulatory framework", "The company's compliance reporting to the board of directors is less frequent than best practices suggest"])}

Finding 3 (MEDIUM RISK): ${rng.pick(["The company's whistleblower hotline is not adequately publicized and does not offer anonymous reporting in all jurisdictions where the company operates", "The company's compliance training curriculum has not been updated to reflect recent regulatory changes and enforcement trends", "Internal controls over financial reporting contain a material weakness related to the segregation of duties in the accounts payable process", "The company's information security program does not include regular penetration testing or vulnerability assessments as recommended by industry standards", "Third-party due diligence procedures are applied inconsistently across business units, with certain higher-risk relationships receiving inadequate scrutiny"])}

Finding 4 (LOW RISK): ${rng.pick(["Documentation of the company's compliance risk assessment methodology could be improved to demonstrate the rationale for risk ratings", "The company's code of conduct has not been updated in three years and does not address several emerging risk areas", "Internal audit has not conducted a dedicated compliance audit within the past 18 months", "The compliance function's headcount has not kept pace with the growth in the company's operations and regulatory footprint"])}

III-A. DETAILED GAP ANALYSIS

Our review compared ${client}'s current compliance program against the ${rng.pick(["DOJ Evaluation of Corporate Compliance Programs framework", "Committee of Sponsoring Organizations (COSO) Internal Control framework", "ISO 37301 Compliance Management System standard", "Federal Sentencing Guidelines criteria for an effective compliance program"])}. The following areas were assessed:

Program Design and Structure: ${rng.pick(["The compliance program has a clear organizational structure with reporting lines to both the General Counsel and the Audit Committee, which is consistent with best practices", "The compliance function is understaffed relative to the company's size and risk profile, with only " + rng.int(2, 8) + " full-time compliance professionals for a " + rng.int(1000, 15000) + "-employee organization", "The Chief Compliance Officer has adequate access to senior management and the board, but the compliance budget has been flat for three consecutive years despite increasing regulatory obligations"])}. ${rng.pick(["The program would benefit from a dedicated compliance technology platform to automate monitoring and reporting", "The compliance function's authority to investigate and remediate issues is clearly defined in the corporate governance documents", "We recommend establishing a cross-functional compliance committee with representatives from legal, finance, operations, and human resources"])}.

Policies and Procedures: The company maintains ${rng.int(15, 60)} compliance-related policies covering key risk areas. Our gap analysis identified ${rng.int(2, 10)} policies that require updates to address ${rng.pick(["recent regulatory changes", "new business activities and geographic expansion", "emerging risk areas such as artificial intelligence and automated decision-making", "feedback from internal and external audits"])}. Policy distribution and acknowledgment tracking is ${rng.pick(["managed through a centralized platform with adequate documentation", "handled manually and does not provide reliable evidence of employee awareness", "automated but does not include periodic re-certification as recommended by the regulatory framework"])}.

Training and Communication: ${rng.pick(["The company provides annual compliance training to all employees, with role-specific modules for higher-risk functions", "Training completion rates average " + rng.int(70, 95) + "% across the organization, with lower rates in field offices", "The training content is primarily delivered through an online platform and has not been updated to include recent enforcement examples", "New hire compliance training is completed within the first " + rng.int(30, 90) + " days of employment, which exceeds the recommended 30-day benchmark"])}. We recommend ${rng.pick(["supplementing the annual training with quarterly compliance updates and targeted communications", "incorporating real-world case studies and enforcement actions into the training curriculum", "implementing competency assessments to verify employee understanding of key compliance requirements", "expanding the compliance awareness program to include regular communications from senior leadership"])}.

IV. RECOMMENDATIONS

We recommend the following remediation plan, prioritized by risk level:

Immediate (0-30 days):
- ${rng.pick(["Engage outside counsel to conduct a privileged assessment of the highest-risk areas", "Implement interim controls to address the critical findings pending development of permanent solutions", "Brief the board of directors and audit committee on the key findings and remediation timeline"])}

Short-term (30-90 days):
- ${rng.pick(["Update all vendor agreements to include required compliance provisions", "Conduct a comprehensive risk assessment following the regulatory framework", "Develop and implement enhanced monitoring and testing procedures"])}

Medium-term (90-180 days):
- ${rng.pick(["Implement a compliance management system to track obligations, deadlines, and remediation activities", "Conduct enterprise-wide training on updated policies and procedures", "Engage an independent auditor to validate the effectiveness of the remediation measures"])}

The estimated cost of the recommended remediation program is $${rng.int(200, 2000)}K, with ongoing annual compliance costs of approximately $${rng.int(100, 800)}K.`,
    };
  },
};

// --- Content expansion ---
// Each generator needs to produce 1000+ words to span multiple 512-word chunks.
// These blocks add substantive, varied legal/business prose.

const PROCEDURAL_HISTORY_BLOCKS = [
  (rng: Rng, plaintiff: string, defendant: string) =>
    `PROCEDURAL HISTORY

On ${rng.pick(["January", "March", "May", "July", "September", "November"])} ${rng.int(1, 28)}, ${rng.int(2023, 2025)}, ${plaintiff} commenced this action by filing a ${rng.pick(["Verified Complaint", "Complaint and Demand for Jury Trial", "Complaint for Declaratory and Injunctive Relief"])} alleging ${rng.int(3, 8)} causes of action against ${defendant}. On ${rng.pick(["February", "April", "June", "August", "October", "December"])} ${rng.int(1, 28)}, ${rng.int(2024, 2025)}, ${defendant} filed ${rng.pick(["a Motion to Dismiss pursuant to Rule 12(b)(6)", "its Answer and Counterclaims", "a Motion to Transfer Venue to the " + rng.pick(COURTS).name])}.

The Court held a ${rng.pick(["telephonic", "in-person"])} scheduling conference on ${rng.pick(["March", "May", "July"])} ${rng.int(1, 28)}, ${rng.int(2024, 2026)}, at which the parties agreed to a ${rng.pick(["phased discovery plan", "bifurcated trial schedule", "mediation referral before the close of fact discovery"])}. The Court entered a Scheduling Order setting the following deadlines: fact discovery to close on ${rng.pick(["September", "October", "November", "December"])} ${rng.int(1, 28)}, ${rng.int(2025, 2026)}; expert reports due ${rng.int(30, 60)} days thereafter; and dispositive motions due ${rng.int(30, 45)} days after the close of expert discovery.

During the course of discovery, the parties have exchanged approximately ${rng.int(50000, 500000).toLocaleString()} documents. ${plaintiff} has taken the depositions of ${rng.int(3, 12)} current and former employees of ${defendant}, including its Chief Executive Officer, Chief Financial Officer, and General Counsel. ${defendant} has deposed ${rng.int(2, 8)} representatives of ${plaintiff}. The parties have also engaged in extensive meet-and-confer proceedings regarding ${rng.pick(["privilege disputes over approximately " + rng.int(500, 5000) + " documents", "the scope of electronically stored information to be produced", "the adequacy of " + defendant + "'s search terms and custodians", "disputed interrogatory responses and requests for admission"])}.

${rng.pick(["The Court has resolved two discovery motions, granting " + plaintiff + "'s motion to compel production of " + defendant + "'s internal communications and denying " + defendant + "'s motion for a protective order.", "No dispositive motions have been filed to date. The matter is now ripe for the instant motion.", "The Court previously denied " + defendant + "'s motion to dismiss in a detailed opinion finding that " + plaintiff + " had adequately pleaded each element of its claims.", "The parties participated in a full-day mediation session before a retired judge, which was unsuccessful. The case is now proceeding toward trial."])}`,
];

const ADDITIONAL_ARGUMENT_BLOCKS = [
  (rng: Rng, topic: string) => {
    const cases = rng.pickN(CASE_LAW, 3);
    return `
II. ${rng.pick(["THE BALANCE OF EQUITIES FAVORS", "PUBLIC POLICY SUPPORTS", "THE WEIGHT OF AUTHORITY COMPELS", "FUNDAMENTAL PRINCIPLES OF FAIRNESS REQUIRE"]).toUpperCase()} ${rng.pick(["GRANTING THE REQUESTED RELIEF", "DENYING THE MOTION", "ENTERING JUDGMENT AS A MATTER OF LAW"]).toUpperCase()}

Even if the Court were to find that the primary legal standard is not satisfied, the ${rng.pick(["balance of equities", "totality of the circumstances", "weight of the competing policy considerations"])} independently supports the relief requested. Courts have consistently recognized that in cases involving ${topic}, the ${rng.pick(["public interest in deterring such conduct outweighs any prejudice to the defendant", "need to protect the integrity of the judicial process requires a robust remedy", "equitable principles of fairness and good conscience demand relief"])}. See ${cases[0].name}, ${cases[0].cite}.

The practical consequences of ${rng.pick(["denying", "granting"])} the present motion are significant. If the Court ${rng.pick(["permits the challenged conduct to continue", "declines to enforce the contractual provisions at issue", "applies the standard advocated by the opposing party"])}, it will ${rng.pick(["effectively reward bad-faith conduct and create perverse incentives for future litigants", "render meaningless the protections that the parties specifically negotiated and agreed to", "undermine the predictability and stability that commercial parties depend on when structuring complex transactions", "create an unjustifiable asymmetry between the rights and obligations of the parties"])}.

Furthermore, the ${rng.pick(["legislative history", "regulatory framework", "body of case law"])} demonstrates a clear trend toward ${rng.pick(["expanding liability in cases involving " + topic, "heightening the standards of conduct applicable to parties in the defendant's position", "providing more robust remedies to plaintiffs who can demonstrate the elements at issue here", "closing the loopholes that defendants in similar cases have historically exploited"])}. As the court noted in ${cases[1].name}, ${cases[1].cite}, "${rng.pick(["the law does not countenance sharp dealing that, while perhaps technically within the letter of the agreement, violates its spirit", "parties who assume fiduciary obligations cannot hide behind procedural technicalities to escape their fundamental duties", "equity will not permit a party to retain the benefits of a transaction while repudiating the obligations that were the consideration for those benefits"])}."

The ${rng.pick(["amicus brief filed by the Securities and Exchange Commission", "Department of Justice Statement of Interest", "Federal Trade Commission's enforcement guidance", "industry group's position paper"])} further supports this conclusion, noting that ${rng.pick(["effective enforcement of these provisions is essential to maintaining public confidence in the markets", "the challenged conduct presents systemic risks that extend beyond the parties to this case", "the regulatory framework was specifically designed to address the type of conduct at issue here", "failure to impose meaningful consequences would send a troubling signal to market participants"])}. See also ${cases[2].name}, ${cases[2].cite} (${rng.pick(["reaching a similar conclusion on analogous facts", "affirming the lower court's entry of the requested relief", "holding that the applicable legal standard requires consideration of these broader policy concerns"])}).`;
  },
];

const STATUTE_ANNOTATIONS = [
  (rng: Rng, statuteName: string) => {
    const cases = rng.pickN(CASE_LAW, 4);
    return `
ANNOTATIONS AND CASE NOTES

1. Construction and Application. The ${rng.pick(["Supreme Court", "Court of Appeals", "majority of courts"])} has construed this provision ${rng.pick(["broadly", "liberally", "in accordance with its remedial purpose"])} to effectuate the ${rng.pick(["legislative intent of protecting investors from fraudulent practices", "congressional purpose of ensuring fair and transparent markets", "statutory objective of preventing abuse of fiduciary relationships", "regulatory goal of maintaining the integrity of financial reporting"])}. ${cases[0].name}, ${cases[0].cite}. However, ${rng.pick(["the provision does not extend to conduct that is merely negligent or inadvertent", "courts have imposed a heightened pleading standard under this section", "the statute of limitations for claims under this provision is strictly enforced", "defendants may invoke the good faith defense if they can demonstrate reasonable reliance on professional advice"])}. ${cases[1].name}, ${cases[1].cite}.

2. Elements of a Claim. To establish a violation of this section, the ${rng.pick(["plaintiff", "Commission", "claimant"])} must prove: (i) ${rng.pick(["a material misrepresentation or omission", "the existence of a duty owed to the plaintiff", "that the defendant engaged in prohibited conduct"])}; (ii) ${rng.pick(["scienter, or a mental state embracing intent to deceive, manipulate, or defraud", "a breach of the applicable standard of care", "that the violation was committed knowingly or with reckless disregard"])}; (iii) ${rng.pick(["a connection with the purchase or sale of a security", "that the plaintiff suffered an injury in fact", "that the defendant's conduct affected interstate commerce"])}; (iv) ${rng.pick(["reliance by the plaintiff on the defendant's misrepresentation", "a causal nexus between the violation and the harm suffered", "that the violation was a proximate cause of the plaintiff's damages"])}; and (v) ${rng.pick(["economic loss", "actual damages", "injury to business or property"])}. ${cases[2].name}, ${cases[2].cite}.

3. Defenses. The following defenses have been recognized under this section: ${rng.pick(["good faith reliance on the advice of counsel", "the in pari delicto doctrine", "ratification by a fully informed disinterested decision-maker", "the safe harbor for forward-looking statements accompanied by meaningful cautionary language"])}. ${cases[3].name}, ${cases[3].cite}. The burden of proving an affirmative defense rests with the defendant, who must demonstrate ${rng.pick(["by a preponderance of the evidence that the defense applies", "that the elements of the defense are satisfied under the specific facts of the case", "both the objective and subjective components of the good faith standard"])}.

4. Remedies. Available remedies include ${rng.pick(["compensatory damages measured by the out-of-pocket loss or benefit-of-the-bargain standard", "disgorgement of ill-gotten gains", "rescission of the transaction", "civil penalties of up to three times the profit gained or loss avoided"])}. Prejudgment interest is ${rng.pick(["available in the discretion of the court", "presumptively awarded unless the defendant demonstrates that it would be inequitable", "calculated from the date of the violation at the applicable Treasury bill rate"])}. Attorneys' fees may be awarded ${rng.pick(["to the prevailing party under the fee-shifting provision", "in cases involving bad faith or vexatious conduct", "where the litigation has resulted in a substantial benefit to a class of persons"])}.

5. Statute of Limitations. Claims under this section must be brought within ${rng.pick(["two years of discovery of the violation and no later than five years after the violation occurred", "the earlier of (a) two years after the date of discovery or (b) five years after the date of the violation", "three years of the date on which the plaintiff knew or should have known of the alleged violation"])}. The discovery rule ${rng.pick(["tolls the limitations period until the plaintiff knew or reasonably should have known of the facts constituting the violation", "requires the plaintiff to exercise reasonable diligence in discovering the basis for the claim", "applies where the defendant has engaged in fraudulent concealment of the material facts"])}.

CROSS REFERENCES

See also: ${rng.pickN(STATUTES, 3).map((s) => `${s.name}, ${s.cite}`).join("; ")}.

PRACTICE NOTES

Practitioners should be aware that ${rng.pick(["the Commission has issued several no-action letters providing guidance on the application of this section to specific fact patterns", "proposed rulemaking is pending that would significantly expand the scope of this provision", "recent enforcement actions suggest an increasingly aggressive posture by the regulatory authorities with respect to this section", "the courts are split on several important interpretive questions, creating uncertainty for parties in different jurisdictions"])}.`;
  },
];

const EXTRA_MEMO_DISCUSSION = [
  (rng: Rng, client: string, opponent: string, topic: string) => {
    const cases = rng.pickN(CASE_LAW, 3);
    return `
D. Damages Analysis

The quantification of damages in this matter involves ${rng.pick(["both direct contractual damages and consequential losses", "complex financial modeling and expert testimony", "multiple theories of recovery that may yield different damage figures", "consideration of both compensatory and potential punitive damages"])}.

Based on our preliminary analysis, ${client}'s damages can be categorized as follows:

Direct Damages: The most straightforward measure of damages is the ${rng.pick(["benefit-of-the-bargain calculation, which compares the value " + client + " expected to receive under the Agreement with what it actually received", "out-of-pocket loss, representing the difference between the amount paid by " + client + " and the value received", "cost-of-completion approach, measuring the cost to " + client + " of obtaining substitute performance from a third party"])}. Our preliminary estimate, based on the available financial data, is approximately $${rng.int(10, 200)} million.

Lost Profits: ${client} may also seek recovery of ${rng.pick(["lost profits attributable to the delay in product launch caused by " + opponent + "'s breach", "profits that " + client + " would have earned from the diverted business opportunities", "the revenue shortfall resulting from " + opponent + "'s failure to perform its marketing and distribution obligations"])}. The lost profits analysis will require expert testimony, and we have identified ${rng.int(2, 4)} potential expert witnesses with relevant industry experience. See ${cases[0].name}, ${cases[0].cite} (${rng.pick(["holding that lost profits are recoverable where they can be proven with reasonable certainty", "requiring a detailed methodology for calculating lost profits in complex commercial disputes", "affirming the admissibility of a discounted cash flow analysis for lost profits"])}).

Consequential Damages: The ${rng.pick(["reputational harm", "disruption to " + client + "'s ongoing business relationships", "regulatory consequences", "loss of key employees and institutional knowledge"])} resulting from ${opponent}'s conduct may give rise to additional consequential damages. While these damages are inherently more difficult to quantify, ${cases[1].name}, ${cases[1].cite}, confirms that they are recoverable where ${rng.pick(["the parties contemplated such losses at the time of contracting", "the damages were a foreseeable consequence of the breach", "the causal chain between the breach and the harm is sufficiently direct"])}.

E. Litigation Risk Assessment

We assess the overall probability of success on the merits at approximately ${rng.int(55, 85)}%, with the following breakdown by claim:

Claim 1 (${topic}): ${rng.int(60, 90)}% likelihood of prevailing. This is our strongest claim, supported by ${rng.pick(["clear documentary evidence", "controlling precedent in this jurisdiction", "the unambiguous contractual language", "compelling testimony from multiple witnesses"])}.

Claim 2 (${rng.pick(["unjust enrichment", "fraudulent concealment", "tortious interference", "breach of implied covenant of good faith"])}): ${rng.int(40, 75)}% likelihood of prevailing. This claim is ${rng.pick(["somewhat weaker due to potential statute of limitations issues", "dependent on the factual findings regarding " + opponent + "'s state of mind", "subject to a pending motion to dismiss that may narrow the available theories", "complicated by the contractual limitation of liability provision"])}. See ${cases[2].name}, ${cases[2].cite}.

The primary litigation risks include: (1) ${rng.pick(["the possibility of an adverse ruling on the pending Daubert motion to exclude our damages expert", "the risk that the court will apply the more restrictive legal standard advocated by " + opponent, "the challenge of proving causation given the multiple potential contributing factors", "the possibility that the jury may find comparative fault on the part of " + client])}; and (2) ${rng.pick(["the significant expense and distraction of a multi-week trial", "the unpredictability of jury deliberations", "the possibility that " + opponent + " will assert counterclaims that could offset any recovery", "the risk that an adverse ruling could establish unfavorable precedent for " + client + "'s other pending matters"])}.

F. Settlement Valuation

Taking into account the probability-adjusted damages analysis and the litigation risks identified above, we estimate the settlement value of this matter at $${rng.int(5, 100)} million, within a range of $${rng.int(2, 40)} million (low) to $${rng.int(50, 200)} million (high). This range reflects ${rng.pick(["the strength of the underlying claims, discounted for litigation risk", "comparable settlements in similar cases within this jurisdiction", "the expected cost of litigation through trial, including expert fees and opportunity costs", "the strategic value of resolving the matter promptly to minimize business disruption"])}.`;
  },
];

const EXTRA_CORRESPONDENCE_BLOCKS = [
  (rng: Rng) => `
We have conducted a thorough review of the relevant documents and communications, including ${rng.pick(["the original Agreement and all amendments thereto", "the complete email correspondence between the parties' respective teams", "the financial records and audit reports for the relevant period", "the board minutes and resolutions authorizing the transaction"])}. Our review has revealed several additional matters that warrant attention:

First, ${rng.pick(["the documentation supporting the claimed damages is incomplete and will require supplementation before formal proceedings can be initiated", "there appear to be additional witnesses who have not yet been interviewed and whose testimony may be material", "the insurance coverage analysis suggests that certain of the claimed losses may be covered under existing policies", "the regulatory implications of the underlying conduct extend beyond the immediate dispute and may require separate attention"])}.

Second, ${rng.pick(["we have identified a potential conflict of interest involving one of the third-party advisors retained during the transaction", "the statute of limitations on certain of the claims will expire within the next 90 days, requiring prompt action", "recent developments in the relevant case law may strengthen our negotiating position", "the opposing party has retained new counsel, which may signal a change in litigation strategy"])}.

Third, ${rng.pick(["the international dimensions of this matter raise additional considerations regarding jurisdiction, choice of law, and enforcement of any judgment", "we recommend engaging a forensic accountant to trace the flow of funds and quantify the damages with greater precision", "the public disclosure requirements associated with this dispute must be carefully managed to avoid adverse market reactions", "there are pending regulatory proceedings involving the same subject matter that could affect the timing and strategy of our case"])}.

We propose scheduling a meeting during the week of ${rng.pick(["March", "April", "May", "June"])} ${rng.int(10, 25)} to discuss these matters in detail and to develop a comprehensive action plan. Please have your team prepare the following materials in advance: (1) ${rng.pick(["a complete timeline of all relevant events and communications", "a summary of all insurance policies that may provide coverage", "an updated analysis of the financial impact on the company", "a list of all individuals with knowledge of the material facts"])}; (2) ${rng.pick(["copies of all board presentations relating to the transaction", "the most recent financial statements and projections", "a summary of all pending or threatened claims relating to the same subject matter", "an organizational chart showing the reporting relationships of all key personnel"])}; and (3) ${rng.pick(["any prior correspondence with regulatory authorities on related matters", "a summary of comparable transactions and their outcomes", "an assessment of the potential reputational impact of various litigation strategies", "a budget and timeline for the recommended course of action"])}.`,
];

const EXTRA_DATA_ANALYSIS = [
  (rng: Rng, company: string) => {
    return `
SENSITIVITY ANALYSIS

The following scenarios illustrate the impact of key assumptions on the valuation:

Scenario 1 (Base Case): Revenue growth of ${rng.int(5, 15)}% with EBITDA margins of ${rng.int(15, 35)}%. Implied enterprise value: $${rng.int(200, 5000)} million. This scenario assumes continuation of current market conditions and successful execution of the company's stated business plan.

Scenario 2 (Upside): Revenue growth of ${rng.int(15, 30)}% driven by ${rng.pick(["successful product launches in new markets", "market share gains from weaker competitors", "favorable regulatory changes", "accelerated digital transformation initiatives"])}. EBITDA margins expand to ${rng.int(25, 45)}% through ${rng.pick(["operating leverage", "cost optimization programs", "favorable product mix shift", "reduced customer acquisition costs"])}. Implied enterprise value: $${rng.int(500, 10000)} million.

Scenario 3 (Downside): Revenue growth of ${rng.int(-5, 5)}% reflecting ${rng.pick(["economic recession impact", "increased competitive pressure", "regulatory headwinds", "customer churn in key segments"])}. EBITDA margins compress to ${rng.int(5, 20)}% due to ${rng.pick(["pricing pressure from competitors", "increased compliance costs", "supply chain disruptions", "higher labor costs"])}. Implied enterprise value: $${rng.int(100, 1000)} million.

INDUSTRY BENCHMARKING

${company}'s performance relative to industry peers:

Revenue Growth: ${company} ranks in the ${rng.pick(["top quartile", "second quartile", "third quartile", "bottom quartile"])} among comparable companies, with ${rng.int(-5, 30)}% growth compared to a peer median of ${rng.int(3, 20)}%. Key drivers of ${rng.pick(["outperformance", "underperformance"])} include ${rng.pick(["strong customer retention and expansion revenue", "successful international expansion", "slower adoption of new product offerings", "market-specific headwinds in key geographies"])}.

Profitability: Operating margins of ${rng.int(5, 40)}% compare to a peer median of ${rng.int(10, 30)}%. The ${rng.pick(["favorable", "unfavorable"])} variance is primarily attributable to ${rng.pick(["higher R&D spending as a percentage of revenue", "more efficient go-to-market strategy", "scale advantages in procurement", "legacy cost structures that have not yet been optimized"])}.

Capital Efficiency: Return on invested capital of ${rng.int(5, 30)}% ${rng.pick(["exceeds", "falls below"])} the weighted average cost of capital of ${rng.int(6, 14)}%, ${rng.pick(["indicating value creation for shareholders", "suggesting potential capital allocation issues", "reflecting the capital-intensive nature of the business", "driven primarily by the asset-light business model"])}.

Balance Sheet: Net debt to EBITDA of ${(rng.int(5, 50) / 10).toFixed(1)}x compares to a peer median of ${(rng.int(15, 40) / 10).toFixed(1)}x. ${rng.pick(["The lower leverage provides financial flexibility for strategic acquisitions", "The higher leverage reflects the recent leveraged buyout transaction", "Leverage is expected to decline as the company generates free cash flow", "The company has adequate liquidity with $" + rng.int(50, 500) + " million in undrawn revolving credit facility"])}.

MANAGEMENT ASSESSMENT

Key observations from management interviews and reference checks:

CEO (${rng.pick(PERSON_NAMES)}): ${rng.pick(["Highly regarded by industry analysts and board members. Strong track record of execution.", "Relatively new in role (appointed " + rng.int(6, 24) + " months ago). Still establishing strategic direction.", "Experienced industry veteran with deep relationships across the value chain.", "Effective operator but limited experience with transformational initiatives."])}

CFO (${rng.pick(PERSON_NAMES)}): ${rng.pick(["Solid financial background with prior experience at a Big Four accounting firm.", "Has successfully led two debt refinancing transactions in the past 18 months.", "Limited experience with public company reporting requirements.", "Strong analytical skills but would benefit from a more experienced treasury team."])}

Board of Directors: ${rng.pick(["Well-constituted board with appropriate independence and expertise.", "Several long-tenured directors may benefit from refreshment.", "Recently added two directors with relevant industry experience.", "Board governance practices are generally consistent with best practices, with minor exceptions noted in the compliance review."])}`;
  },
];

// --- Document generators ---

function generateContract(id: number, rng: Rng): GeneratedDoc {
  const contractType = rng.pick(CONTRACT_TYPES);
  const party1 = rng.pick(COMPANY_NAMES);
  const party2 = rng.pick(COMPANY_NAMES.filter((c) => c !== party1));
  const jurisdiction = rng.pick(JURISDICTIONS);
  const effectiveDate = `${rng.pick(["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"])} ${rng.int(1, 28)}, ${rng.int(2020, 2026)}`;
  const privileged = rng.chance(0.15);

  const selectedClauses = rng.pickN(Object.keys(CONTRACT_CLAUSES) as (keyof typeof CONTRACT_CLAUSES)[], rng.int(6, 10));
  const recitals = rng.pickN(CONTRACT_RECITALS, rng.int(2, 4));

  let body = `${contractType.toUpperCase()}

This ${contractType} (the "Agreement") is entered into as of ${effectiveDate} (the "Effective Date"), by and between:

${party1}, a ${rng.pick(["Delaware corporation", "New York limited liability company", "California corporation"])} with its principal place of business at ${rng.int(100, 9999)} ${rng.pick(["Broadway", "Park Avenue", "Market Street", "Michigan Avenue", "Constitution Avenue"])}${rng.pick([", Suite " + rng.int(100, 5000), ""])}, ${rng.pick(["New York, NY", "San Francisco, CA", "Chicago, IL", "Houston, TX", "Washington, DC"])} (the "${rng.pick(["Company", "Licensor", "Seller", "Employer", "Disclosing Party"])}");

and

${party2}, a ${rng.pick(["Delaware limited liability company", "corporation organized under the laws of Nevada", "Texas limited partnership"])} with its principal place of business at ${rng.int(100, 9999)} ${rng.pick(["Main Street", "Elm Street", "Oak Avenue", "Industrial Boulevard", "Commerce Drive"])}${rng.pick([", Floor " + rng.int(2, 40), ""])}, ${rng.pick(["Los Angeles, CA", "Dallas, TX", "Boston, MA", "Seattle, WA", "Miami, FL"])} (the "${rng.pick(["Client", "Licensee", "Buyer", "Employee", "Receiving Party"])}").

Each a "Party" and together, the "Parties."

RECITALS

${recitals.join("\n\n")}

NOW, THEREFORE, in consideration of the mutual covenants and agreements set forth herein, and for other good and valuable consideration, the receipt and sufficiency of which are hereby acknowledged, the Parties agree as follows:

`;

  selectedClauses.forEach((clauseKey, i) => {
    let clause = CONTRACT_CLAUSES[clauseKey];
    clause = clause.replace(/\{\{jurisdiction\}\}/g, jurisdiction);
    body += `ARTICLE ${i + 1}. ${clause}\n\n`;
  });

  body += `IN WITNESS WHEREOF, the Parties have executed this Agreement as of the date first written above.

${party1.toUpperCase()}

By: _________________________
Name: ${rng.pick(PERSON_NAMES)}
Title: ${rng.pick(["Chief Executive Officer", "President", "General Counsel", "Chief Financial Officer", "Executive Vice President"])}

${party2.toUpperCase()}

By: _________________________
Name: ${rng.pick(PERSON_NAMES)}
Title: ${rng.pick(["Chief Executive Officer", "Managing Director", "General Counsel", "Chief Operating Officer", "Senior Vice President"])}`;

  const uniqueMarker = `contract-${id}-${contractType.toLowerCase().replace(/\s+/g, "-")}`;

  return {
    file_name: `${contractType.toLowerCase().replace(/\s+/g, "_")}_${party1.split(" ")[0].toLowerCase()}_${party2.split(" ")[0].toLowerCase()}_${id}.md`,
    body,
    doc_type: "contract",
    jurisdiction,
    privileged,
    ground_truth: {
      unique_markers: [uniqueMarker],
      topics: selectedClauses.map(String),
      citations: [],
      entities: [party1, party2],
      tags: ["contract", contractType.toLowerCase()],
    },
  };
}

function generateFiling(id: number, rng: Rng): GeneratedDoc {
  const filingType = rng.pick([
    "Complaint",
    "Motion to Dismiss",
    "Motion for Summary Judgment",
    "Motion for Preliminary Injunction",
    "Opposition Brief",
    "Reply Brief",
    "Memorandum of Law",
    "Motion to Compel Discovery",
    "Motion for Class Certification",
    "Motion in Limine",
  ]);
  const topic = rng.pick(LEGAL_TOPICS);
  const court = rng.pick(COURTS);
  const plaintiff = rng.pick(COMPANY_NAMES);
  const defendant = rng.pick(COMPANY_NAMES.filter((c) => c !== plaintiff));
  const caseNo = `${rng.int(20, 26)}-cv-${String(rng.int(1, 9999)).padStart(4, "0")}`;
  const judge = `Hon. ${rng.pick(PERSON_NAMES)}`;
  const firm = rng.pick(LAW_FIRMS);
  const attorney = rng.pick(PERSON_NAMES);
  const jurisdiction = court.name.includes("Delaware")
    ? "Delaware"
    : court.name.includes("New York")
      ? "New York"
      : court.name.includes("California")
        ? "California"
        : court.name.includes("Texas")
          ? "Texas"
          : rng.pick(JURISDICTIONS);

  const cases = rng.pickN(CASE_LAW, rng.int(3, 6));
  const statutes = rng.pickN(STATUTES, rng.int(1, 3));

  let body = `UNITED STATES DISTRICT COURT
${court.name.toUpperCase()}

${plaintiff},
                              Plaintiff,         Case No. ${caseNo}
          v.
                                                  ${judge}
${defendant},
                              Defendant.

${filingType.toUpperCase()}

${plaintiff}, by and through its undersigned counsel, respectfully submits this ${filingType} and states as follows:

PRELIMINARY STATEMENT

This action arises from ${defendant}'s ${rng.pick(["systematic and deliberate", "knowing and willful", "reckless and continuing"])} ${topic}. Through a course of conduct spanning ${rng.pick(["several months", "more than a year", "the entirety of the parties' business relationship"])}, ${defendant} ${rng.pick(["engaged in a scheme to defraud " + plaintiff + " of millions of dollars", "breached its contractual obligations and fiduciary duties to " + plaintiff, "misappropriated " + plaintiff + "'s proprietary technology and confidential business information", "systematically violated the regulatory requirements designed to protect entities in " + plaintiff + "'s position"])}.

${FILING_SECTIONS.statement_of_facts(rng, plaintiff, defendant, topic)}

${PROCEDURAL_HISTORY_BLOCKS[0](rng, plaintiff, defendant)}

${FILING_SECTIONS.legal_argument(rng, topic)}

${ADDITIONAL_ARGUMENT_BLOCKS[0](rng, topic)}

${FILING_SECTIONS.prayer_for_relief(rng, plaintiff)}

Respectfully submitted,

${firm}

By: /s/ ${attorney}
    ${attorney}
    ${firm}
    ${rng.int(100, 999)} ${rng.pick(["Lexington Avenue", "K Street NW", "Montgomery Street", "LaSalle Street"])}
    ${rng.pick(["New York, NY 10022", "Washington, DC 20006", "San Francisco, CA 94104", "Chicago, IL 60603"])}
    Attorneys for ${plaintiff}`;

  return {
    file_name: `${filingType.toLowerCase().replace(/\s+/g, "_")}_${caseNo.replace(/[^a-z0-9]/gi, "")}_${id}.md`,
    body,
    doc_type: "filing",
    jurisdiction,
    privileged: false,
    ground_truth: {
      unique_markers: [caseNo, `filing-${id}`],
      topics: [topic, filingType.toLowerCase()],
      citations: [
        ...cases.map((c) => c.cite),
        ...statutes.map((s) => s.cite),
      ],
      entities: [plaintiff, defendant, firm, attorney],
      tags: ["filing", filingType.toLowerCase().replace(/\s+/g, "_")],
    },
  };
}

function generateMemo(id: number, rng: Rng): GeneratedDoc {
  const template = rng.chance(0.5)
    ? MEMO_TEMPLATES.case_analysis(rng)
    : MEMO_TEMPLATES.regulatory_analysis(rng);

  const jurisdiction = rng.pick(JURISDICTIONS);

  return {
    file_name: `memo_${id}_${template.title.slice(0, 40).replace(/[^a-z0-9]/gi, "_").toLowerCase()}.md`,
    body: template.body,
    doc_type: "memo",
    jurisdiction,
    privileged: rng.chance(0.6),
    ground_truth: {
      unique_markers: [`memo-${id}`],
      topics: [template.title],
      citations: [],
      entities: [],
      tags: ["memo"],
    },
  };
}

function generateStatute(id: number, rng: Rng): GeneratedDoc {
  const statute = rng.pick(STATUTES);
  const jurisdiction = statute.cite.includes("Del.")
    ? "Delaware"
    : statute.cite.includes("N.Y.")
      ? "New York"
      : statute.cite.includes("Cal.")
        ? "California"
        : statute.cite.includes("U.S.C.") || statute.cite.includes("C.F.R.")
          ? "Federal"
          : rng.pick(JURISDICTIONS);

  const sectionNum = rng.int(1, 50);
  const subsections = rng.int(3, 8);

  let body = `${statute.name}
${statute.cite}

Section ${sectionNum}. ${rng.pick(["Definitions", "Scope and Applicability", "Prohibited Conduct", "Required Disclosures", "Enforcement and Remedies", "Safe Harbor Provisions", "Exemptions", "Civil Liability", "Administrative Proceedings", "Statute of Limitations"])}

(a) ${rng.pick(["For purposes of this section, the following definitions apply:", "This section shall apply to any person who:", "It shall be unlawful for any person to:", "The Commission shall have the authority to:", "No person shall be held liable under this section if:"])}

`;

  for (let i = 0; i < subsections; i++) {
    const sub = String.fromCharCode(98 + i); // b, c, d, ...
    body += `(${sub}) ${rng.pick([
      `Any person who willfully violates any provision of this section shall be subject to a civil penalty of not more than $${rng.int(10, 500)},000 for each violation, and not more than $${rng.int(1, 10)} million in the aggregate for a related series of violations.`,
      `The term "${rng.pick(["covered entity", "reporting person", "qualified institutional buyer", "accredited investor", "control person"])}" means any ${rng.pick(["corporation, partnership, limited liability company, or other business entity that", "individual or entity that, directly or indirectly,", "person, as defined in Section 3(a)(9),"])} ${rng.pick(["is engaged in interstate commerce and has total assets exceeding $10 million", "exercises control over the management or policies of a covered entity", "beneficially owns more than 5% of any class of equity securities", "has been designated by the Commission pursuant to subsection (f)"])}.`,
      `Nothing in this section shall be construed to ${rng.pick(["create a private right of action", "preempt any State law that provides greater protection", "limit the authority of any Federal or State regulatory agency", "apply to transactions that are subject to the exclusive jurisdiction of another regulatory body", "require disclosure of information that is protected by the attorney-client privilege or the work product doctrine"])}.`,
      `The Commission may, by rule or regulation, ${rng.pick(["exempt any class of persons or transactions from all or any part of the requirements of this section", "prescribe the form and content of any disclosure required under this section", "establish procedures for the filing and processing of complaints under this section", "impose additional requirements as the Commission deems necessary or appropriate in the public interest"])}.`,
      `Any person adversely affected by a final order of the Commission under this section may obtain review of such order in the ${rng.pick(["United States Court of Appeals for the circuit in which the petitioner resides", "United States Court of Appeals for the District of Columbia Circuit", "appropriate Federal district court"])} by filing a petition for review within ${rng.int(30, 90)} days after the date of the order.`,
    ])}

`;
  }

  body += `HISTORICAL AND STATUTORY NOTES

This section was ${rng.pick(["originally enacted as part of the", "added by", "amended by"])} ${rng.pick(["Securities Act of 1933", "Securities Exchange Act of 1934", "Sarbanes-Oxley Act of 2002", "Dodd-Frank Wall Street Reform and Consumer Protection Act of 2010", "JOBS Act of 2012", "Uniform Commercial Code (1952, as amended)"])}. ${rng.pick(["It has been subsequently amended on three occasions.", "The current version incorporates amendments through 2023.", "The Commission has issued interpretive guidance under this section."])}

${STATUTE_ANNOTATIONS[0](rng, statute.name)}`;

  return {
    file_name: `statute_${statute.cite.replace(/[^a-z0-9]/gi, "_").toLowerCase()}_${id}.md`,
    body,
    doc_type: "statute",
    jurisdiction,
    privileged: false,
    ground_truth: {
      unique_markers: [`statute-${id}`, statute.cite],
      topics: [statute.name],
      citations: [statute.cite],
      entities: [],
      tags: ["statute", jurisdiction.toLowerCase()],
    },
  };
}

function generateCorrespondence(id: number, rng: Rng): GeneratedDoc {
  const sender = rng.pick(PERSON_NAMES);
  const recipient = rng.pick(PERSON_NAMES.filter((p) => p !== sender));
  const senderFirm = rng.pick(LAW_FIRMS);
  const recipientCompany = rng.pick(COMPANY_NAMES);
  const topic = rng.pick(LEGAL_TOPICS);
  const jurisdiction = rng.pick(JURISDICTIONS);
  const privileged = rng.chance(0.4);

  const letterType = rng.pick([
    "demand_letter",
    "settlement_proposal",
    "client_update",
    "cease_and_desist",
    "engagement_letter",
  ]);

  let body: string;
  const date = `${rng.pick(["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"])} ${rng.int(1, 28)}, ${rng.int(2024, 2026)}`;

  switch (letterType) {
    case "demand_letter":
      body = `${senderFirm}
${rng.int(100, 999)} ${rng.pick(["Park Avenue", "K Street", "Market Street", "Michigan Avenue"])}
${rng.pick(["New York, NY 10022", "Washington, DC 20006", "San Francisco, CA 94104"])}

${date}

VIA CERTIFIED MAIL AND EMAIL

${recipient}
General Counsel
${recipientCompany}

Re: Demand for Payment — ${topic}

Dear ${recipient.split(" ").pop()}:

This firm represents ${rng.pick(COMPANY_NAMES.filter((c) => c !== recipientCompany))} in connection with the above-referenced matter. We write to demand immediate payment of $${rng.int(1, 50)} million in damages arising from ${recipientCompany}'s ${topic}.

As you are aware, ${recipientCompany} entered into a ${rng.pick(["Master Services Agreement", "License Agreement", "Supply Agreement"])} with our client on ${rng.pick(["January", "March", "June", "September"])} ${rng.int(1, 28)}, ${rng.int(2022, 2024)}. ${recipientCompany} has materially breached that agreement by ${rng.pick(["failing to make the required payments when due", "misappropriating our client's confidential information and trade secrets", "diverting business opportunities in violation of the non-competition provisions", "failing to deliver the contracted goods and services in accordance with the agreed specifications"])}.

Our client has sustained substantial damages as a result of ${recipientCompany}'s conduct, including but not limited to: (1) direct contractual damages of approximately $${rng.int(2, 20)} million; (2) lost profits estimated at $${rng.int(5, 30)} million; and (3) consequential damages related to ${rng.pick(["disruption of our client's business operations", "damage to our client's reputation and customer relationships", "costs incurred in mitigating the effects of the breach"])}.

We demand that ${recipientCompany} cure its breach and pay the full amount of damages within thirty (30) days of the date of this letter. If ${recipientCompany} fails to satisfy this demand, our client is prepared to commence litigation in the ${rng.pick(COURTS).name} without further notice.

This letter is without prejudice to any of our client's rights and remedies, all of which are expressly reserved.

Very truly yours,

${sender}
Partner
${senderFirm}`;
      break;

    case "settlement_proposal":
      body = `${senderFirm}

${date}

CONFIDENTIAL — SETTLEMENT COMMUNICATION
PROTECTED UNDER FRE 408

${recipient}
${rng.pick(LAW_FIRMS.filter((f) => f !== senderFirm))}

Re: Settlement Proposal — ${rng.pick(COMPANY_NAMES)} v. ${recipientCompany}

Dear ${recipient.split(" ").pop()}:

Further to our recent discussions, I write to present a formal settlement proposal on behalf of our client. This proposal is made in the spirit of compromise and is intended to resolve all outstanding claims between the parties without the expense and uncertainty of continued litigation.

Our client proposes the following terms:

1. Payment: ${recipientCompany} shall pay the sum of $${rng.int(1, 25)} million within ${rng.int(30, 90)} days of the execution of a definitive settlement agreement.

2. Release: The parties shall exchange mutual general releases of all claims arising out of or related to the subject matter of the litigation.

3. ${rng.pick(["Injunctive Relief: " + recipientCompany + " shall agree to a consent order prohibiting the challenged conduct for a period of " + rng.int(2, 5) + " years.", "Non-Disparagement: The parties shall agree to mutual non-disparagement obligations.", "Confidentiality: The terms of the settlement shall remain confidential, and the parties shall issue a jointly-approved public statement."])}

4. ${rng.pick(["License: " + recipientCompany + " shall receive a non-exclusive license to the disputed technology on commercially reasonable terms.", "Transition Period: The parties shall cooperate during a " + rng.int(3, 12) + "-month transition period to ensure an orderly wind-down of the business relationship.", "Compliance Monitoring: " + recipientCompany + " shall engage an independent compliance monitor for a period of " + rng.int(1, 3) + " years."])}

This offer will remain open for fifteen (15) business days. We believe these terms represent a fair resolution and look forward to your response.

Regards,

${sender}
${senderFirm}`;
      break;

    default:
      body = `${senderFirm}

${date}

${privileged ? "PRIVILEGED AND CONFIDENTIAL\nATTORNEY-CLIENT COMMUNICATION\n\n" : ""}${recipient}
${recipientCompany}

Re: Status Update — ${topic}

Dear ${recipient.split(" ").pop()}:

I am writing to provide you with an update on the status of the ${topic} matter.

Since our last communication, the following developments have occurred:

1. We have completed our review of the ${rng.pick(["document production", "deposition transcripts", "expert reports", "regulatory filings"])} and have identified several ${rng.pick(["favorable data points that support our position", "areas of concern that will require additional analysis", "documents that may be subject to privilege claims", "potential witnesses who should be interviewed"])}.

2. ${rng.pick(["Opposing counsel has indicated a willingness to discuss settlement", "The court has scheduled a status conference for next month", "We have filed the required responsive pleading", "The discovery deadline has been extended by " + rng.int(30, 90) + " days"])}.

3. Our preliminary assessment of the ${rng.pick(["damages", "liability exposure", "regulatory risk", "litigation timeline"])} indicates that ${rng.pick(["the matter is developing favorably", "there are both strengths and weaknesses in our position", "we should consider adjusting our strategy", "additional resources may be needed"])}.

Next Steps:
- ${rng.pick(["Schedule a meeting to discuss the litigation strategy", "Prepare for the upcoming depositions", "Engage an expert witness in the relevant field", "Draft a motion to compel the outstanding discovery responses"])}
- ${rng.pick(["Review the amended complaint for any new theories", "Analyze the recently produced financial records", "Prepare a privilege log for the contested documents", "Coordinate with the company's insurance carrier"])}

Please do not hesitate to contact me if you have any questions or would like to discuss this matter further.

Best regards,

${sender}
${senderFirm}`;
  }

  body += EXTRA_CORRESPONDENCE_BLOCKS[0](rng);

  // add additional substantive paragraphs
  body += `

In addition to the foregoing, we wish to draw your attention to the following considerations that may affect the overall strategy and timeline:

The ${rng.pick(["regulatory environment", "competitive landscape", "market conditions", "political climate"])} has ${rng.pick(["shifted significantly", "continued to evolve", "become increasingly uncertain"])} since the inception of this matter. ${rng.pick(["Recent enforcement actions by the " + rng.pick(["SEC", "DOJ", "FTC", "CFPB"]) + " suggest heightened scrutiny of conduct similar to that at issue here", "Several comparable disputes have been resolved through " + rng.pick(["arbitration", "mediation", "expedited trial procedures"]) + ", which may offer a more efficient path to resolution", "Legislative proposals currently under consideration could materially alter the legal framework applicable to this dispute", "Industry developments, including " + rng.pick(["recent M&A activity", "technological disruption", "supply chain restructuring", "workforce reduction trends"]) + ", provide additional context for evaluating the claims"])}.

From a financial perspective, the ${rng.pick(["total cost of litigation through trial is estimated at $" + rng.int(2, 15) + " million, inclusive of expert fees and e-discovery costs", "opportunity cost of management time diverted to this matter is substantial and should be factored into any settlement analysis", "potential liability exposure, when probability-weighted, supports a settlement range of $" + rng.int(5, 50) + " million to $" + rng.int(50, 200) + " million", "company's insurance coverage may offset a significant portion of the defense costs and potential settlement, subject to deductible and co-insurance provisions"])}. We recommend ${rng.pick(["engaging a litigation finance advisor to evaluate third-party funding options", "conducting a cost-benefit analysis comparing the expected trial outcome to available settlement terms", "updating the litigation reserve to reflect current exposure assessments", "briefing the audit committee on the financial implications of the various strategic options"])}.

Finally, we note that ${rng.pick(["the statute of limitations on certain related claims will expire within the next " + rng.int(60, 180) + " days, which may necessitate the filing of a protective complaint", "the opposing party's recent public statements may create additional leverage in negotiations", "the court's recent ruling in a related case may provide favorable precedent for our position", "the insurance carrier has indicated that it may decline to renew coverage unless this matter is resolved by the next policy anniversary date"])}. We will continue to monitor these developments and will provide supplemental analysis as warranted.`;

  return {
    file_name: `correspondence_${letterType}_${id}.md`,
    body,
    doc_type: "document",
    jurisdiction,
    privileged,
    ground_truth: {
      unique_markers: [`correspondence-${id}`],
      topics: [topic, letterType.replace(/_/g, " ")],
      citations: [],
      entities: [sender, recipient, senderFirm, recipientCompany],
      tags: ["correspondence", letterType],
    },
  };
}

function generateDataReport(id: number, rng: Rng): GeneratedDoc {
  const reportType = rng.pick([
    "financial_analysis",
    "market_research",
    "due_diligence",
    "risk_assessment",
    "portfolio_review",
  ]);
  const company = rng.pick(COMPANY_NAMES);
  const analyst = rng.pick(PERSON_NAMES);
  const jurisdiction = rng.pick(JURISDICTIONS);

  const quarters = ["Q1", "Q2", "Q3", "Q4"];
  const years = [2023, 2024, 2025];

  let body = `${reportType.replace(/_/g, " ").toUpperCase()} REPORT

Prepared by: ${analyst}
Date: ${rng.pick(["January", "March", "June", "September", "December"])} ${rng.int(2024, 2026)}
Subject: ${company}
Classification: ${rng.pick(["Internal", "Confidential", "Restricted"])}

EXECUTIVE SUMMARY

This report presents a comprehensive ${reportType.replace(/_/g, " ")} of ${company}, covering the period from ${rng.pick(quarters)} ${rng.pick(years)} through ${rng.pick(quarters)} ${rng.pick(years)}. Key findings include:

`;

  switch (reportType) {
    case "financial_analysis":
      body += `- Revenue grew ${rng.int(-5, 35)}% year-over-year to $${rng.int(50, 5000)} million
- EBITDA margin ${rng.pick(["expanded", "contracted"])} by ${rng.int(1, 8)} percentage points to ${rng.int(8, 45)}%
- Free cash flow ${rng.pick(["improved", "declined"])} to $${rng.int(10, 500)} million
- Net debt / EBITDA ratio stands at ${(rng.int(5, 60) / 10).toFixed(1)}x
- Working capital position ${rng.pick(["remains strong", "has deteriorated", "is adequate"])} at $${rng.int(20, 300)} million

DETAILED ANALYSIS

1. Revenue Breakdown

| Segment | Revenue ($M) | Growth (%) | Margin (%) |
|---------|-------------|-----------|-----------|
| ${rng.pick(["Enterprise Software", "Cloud Services", "Professional Services", "Licensing"])} | ${rng.int(50, 2000)} | ${rng.int(-10, 40)} | ${rng.int(15, 65)} |
| ${rng.pick(["Hardware", "Managed Services", "Consulting", "Data Analytics"])} | ${rng.int(20, 800)} | ${rng.int(-15, 30)} | ${rng.int(10, 50)} |
| ${rng.pick(["Support & Maintenance", "Subscriptions", "Implementation", "Training"])} | ${rng.int(10, 400)} | ${rng.int(-5, 25)} | ${rng.int(20, 75)} |

2. Key Financial Metrics

The company's ${rng.pick(["debt service coverage ratio of " + (rng.int(10, 35) / 10).toFixed(1) + "x exceeds the minimum covenant requirement of 1.5x", "current ratio of " + (rng.int(10, 30) / 10).toFixed(1) + " indicates adequate short-term liquidity", "return on invested capital of " + rng.int(5, 25) + "% compares favorably to the weighted average cost of capital of " + rng.int(6, 12) + "%", "days sales outstanding of " + rng.int(30, 90) + " days is within the industry benchmark range"])}.

3. Capital Structure

Total capitalization: $${rng.int(200, 10000)} million
- Senior secured debt: $${rng.int(50, 2000)} million (${rng.pick(["SOFR + " + rng.int(150, 450) + "bps", rng.int(3, 8) + "." + rng.int(0, 9) + "% fixed rate"])})
- Subordinated notes: $${rng.int(25, 500)} million (${rng.int(5, 12)}% coupon, maturing ${rng.int(2026, 2032)})
- Preferred equity: $${rng.int(0, 200)} million
- Common equity: $${rng.int(100, 5000)} million

4. Comparable Company Analysis

| Company | EV/Revenue | EV/EBITDA | P/E |
|---------|-----------|----------|-----|
| Peer 1 | ${(rng.int(15, 120) / 10).toFixed(1)}x | ${(rng.int(50, 250) / 10).toFixed(1)}x | ${(rng.int(100, 400) / 10).toFixed(1)}x |
| Peer 2 | ${(rng.int(15, 120) / 10).toFixed(1)}x | ${(rng.int(50, 250) / 10).toFixed(1)}x | ${(rng.int(100, 400) / 10).toFixed(1)}x |
| Peer 3 | ${(rng.int(15, 120) / 10).toFixed(1)}x | ${(rng.int(50, 250) / 10).toFixed(1)}x | ${(rng.int(100, 400) / 10).toFixed(1)}x |
| ${company} | ${(rng.int(15, 120) / 10).toFixed(1)}x | ${(rng.int(50, 250) / 10).toFixed(1)}x | ${(rng.int(100, 400) / 10).toFixed(1)}x |`;
      break;

    case "due_diligence":
      body += `- ${rng.int(2, 8)} material issues identified requiring further investigation
- ${rng.int(0, 3)} potential deal-breakers flagged for principal review
- Estimated purchase price adjustment: $${rng.int(-50, 50)} million
- ${rng.int(5, 15)} key contracts reviewed with ${rng.int(0, 5)} change-of-control provisions identified

DUE DILIGENCE FINDINGS

Category 1: Corporate and Organizational
- The target is a ${rng.pick(["Delaware corporation", "Nevada LLC", "Cayman Islands exempted company"])} in good standing
- ${rng.pick(["Capitalization table is clean with no undisclosed equity interests", "Several outstanding options and warrants may be subject to acceleration upon change of control", "The company's organizational documents contain unusual provisions regarding director removal"])}
- ${rng.pick(["Board minutes for the past 3 years have been reviewed with no material concerns", "Minutes indicate that certain related-party transactions were not properly disclosed", "The company has been operating without a quorum for the past 6 months"])}

Category 2: Material Contracts
- ${rng.int(20, 200)} contracts reviewed, representing ${rng.int(60, 95)}% of revenue
- ${rng.int(0, 10)} contracts contain change-of-control provisions that may ${rng.pick(["require consent", "trigger termination rights", "accelerate payment obligations"])}
- Key customer concentration risk: top ${rng.int(3, 5)} customers represent ${rng.int(30, 70)}% of total revenue
- ${rng.pick(["No material litigation or dispute provisions triggered", "Several vendors have disputed outstanding invoices totaling $" + rng.int(1, 10) + " million", "A key supply agreement expires within 6 months with no renewal option"])}

Category 3: Intellectual Property
- ${rng.int(5, 50)} patents, ${rng.int(10, 100)} trademarks, and ${rng.int(1, 20)} registered copyrights identified
- ${rng.pick(["All key IP is owned by the target with no encumbrances", "Several patents are jointly owned with a former partner, creating potential licensing issues", "The company relies heavily on trade secrets rather than patents for IP protection"])}
- ${rng.pick(["No pending IP litigation", rng.int(1, 3) + " pending patent infringement claims with aggregate exposure of $" + rng.int(5, 50) + " million", "A cease-and-desist letter was received regarding a trademark dispute"])}

Category 4: Regulatory and Compliance
- ${rng.pick(["The company appears to be in material compliance with applicable regulations", "Several areas of regulatory non-compliance were identified that require remediation", "The company is subject to a pending regulatory investigation"])}
- ${rng.pick(["No material environmental liabilities identified", "Potential environmental remediation costs of $" + rng.int(1, 20) + " million at two former operating sites", "The company's facilities are located in a designated environmental cleanup zone"])}
- Estimated regulatory remediation costs: $${rng.int(0, 15)} million`;
      break;

    default:
      body += `- Overall risk rating: ${rng.pick(["LOW", "MODERATE", "ELEVATED", "HIGH"])}
- ${rng.int(3, 10)} risk factors identified across ${rng.int(2, 5)} categories
- ${rng.int(0, 3)} risks rated as critical, ${rng.int(1, 4)} as high, ${rng.int(2, 6)} as medium
- Recommended risk mitigation budget: $${rng.int(1, 20)} million annually

RISK FACTOR ANALYSIS

Market Risk: ${rng.pick(["LOW", "MODERATE", "HIGH"])}
The company operates in a ${rng.pick(["highly competitive", "moderately concentrated", "rapidly evolving"])} market with ${rng.pick(["significant barriers to entry", "low switching costs for customers", "strong network effects", "cyclical demand patterns"])}. Key market risks include ${rng.pick(["technological disruption from emerging competitors", "customer concentration in a small number of large accounts", "sensitivity to macroeconomic conditions and interest rate changes", "regulatory uncertainty in key markets"])}.

Operational Risk: ${rng.pick(["LOW", "MODERATE", "HIGH"])}
${rng.pick(["The company's operational infrastructure is well-maintained and redundant", "Several single points of failure were identified in the company's supply chain", "The company's IT systems are aging and require significant capital investment", "Key person risk is elevated due to reliance on a small number of technical experts"])}. Mitigation measures include ${rng.pick(["business continuity planning and regular disaster recovery testing", "diversification of supplier relationships", "implementation of enterprise risk management framework", "succession planning for critical roles"])}.

Legal and Regulatory Risk: ${rng.pick(["LOW", "MODERATE", "HIGH"])}
${rng.pick(["The company has no material pending or threatened litigation", "There are " + rng.int(2, 8) + " pending lawsuits with aggregate exposure of $" + rng.int(10, 100) + " million", "The company is subject to a consent decree requiring ongoing compliance monitoring", "Recent regulatory changes may require significant modifications to the company's business practices"])}.`;
  }

  body += EXTRA_DATA_ANALYSIS[0](rng, company);

  return {
    file_name: `report_${reportType}_${company.split(" ")[0].toLowerCase()}_${id}.md`,
    body,
    doc_type: "data",
    jurisdiction,
    privileged: rng.chance(0.3),
    ground_truth: {
      unique_markers: [`report-${id}`],
      topics: [reportType.replace(/_/g, " "), company],
      citations: [],
      entities: [company, analyst],
      tags: ["report", reportType],
    },
  };
}

// --- Public API ---

const GENERATORS: ((id: number, rng: Rng) => GeneratedDoc)[] = [
  generateContract,
  generateContract,    // 2x weight — contracts are most common
  generateFiling,
  generateFiling,      // 2x weight — filings are second most common
  generateMemo,
  generateStatute,
  generateCorrespondence,
  generateDataReport,
];

export function generateDocument(docId: number): GeneratedDoc {
  const rng = new Rng(docId * 2654435761); // golden ratio hash for good distribution
  const gen = GENERATORS[docId % GENERATORS.length];
  return gen(docId, rng);
}

export function generateBatch(startId: number, count: number): GeneratedDoc[] {
  const docs: GeneratedDoc[] = [];
  for (let i = 0; i < count; i++) {
    docs.push(generateDocument(startId + i));
  }
  return docs;
}
