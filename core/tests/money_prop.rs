//! Money rule, as a property test over random job trees.
//!
//! A small model of the contract protocol drives the real `Ledger` and
//! `transition` code through random hire / deliver / check / buy / settle
//! sequences, with random transfer failures. After every step:
//!
//! 1. `sum(real token balances in the tree) + paid_out == client funding + lead bond`
//! 2. each live job's internal ledger matches its real balance (a parent's
//!    ledger may lag by what not-yet-closed children already sent back).
//!
//! At the end everything settles bottom-up and the tree is empty. The root
//! carries a random lead fee: the lead's take is exactly
//! `fee * paid / max(hired, floor)` over the root's own closed children
//! (`floor`: half the work budget; never more than the fee) plus its
//! bond, and the rest goes back to the client.
//! Cases: `PROP_CASES` (default 10,000).

use ht_core::money::{earned_fee, fee_floor, mul_div_floor, note_quote};
use ht_core::{transition, Event, Ledger, Split, Status};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    Hire {
        parent: usize,
        amount: u64,
        sub: u64,
        fail: bool,
    },
    Deliver(usize),
    Check(usize, bool),
    Buy(usize),
    Settle {
        node: usize,
        payee_ok: bool,
        rest_ok: bool,
        /// A routed payee leg whose receiver kept only this share (per
        /// mille) of the pay; the token sent the rest back.
        part: Option<u16>,
    },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (any::<usize>(), 1u64..=5_000_000, 0u64..=3_000_000, prop::bool::weighted(0.15))
            .prop_map(|(parent, amount, sub, fail)| Op::Hire { parent, amount, sub, fail }),
        2 => any::<usize>().prop_map(Op::Deliver),
        2 => (any::<usize>(), prop::bool::weighted(0.8)).prop_map(|(n, p)| Op::Check(n, p)),
        1 => any::<usize>().prop_map(Op::Buy),
        2 => (
            any::<usize>(),
            prop::bool::weighted(0.85),
            prop::bool::weighted(0.85),
            prop::option::weighted(0.2, 0u16..=1000),
        )
            .prop_map(|(node, payee_ok, rest_ok, part)| Op::Settle { node, payee_ok, rest_ok, part }),
    ]
}

struct Node {
    parent: Option<usize>,
    ledger: Ledger,
    status: Status,
    real: u128,
    closed: bool,
    children_open: u32,
    /// Sent to the parent so far (credited to the parent at close).
    returned: u128,
    note_sold: bool,
    /// A settle has started: like the contract (hire needs an open window,
    /// settle needs a closed one), no more hires.
    settle_started: bool,
    /// What the parent funded it with (its price, for the root's fee).
    total: u128,
}

struct World {
    nodes: Vec<Node>,
    funding: u128,
    bond: u128,
    paid_out: u128,
    worker_got: u128,
    treasury_spent: u128,
    treasury_got: u128,
    note_face: u128,
    note_fees: u128,
    /// The root's fee cap, and what the lead (root payee) was paid.
    fee: u128,
    lead_got: u128,
    /// Summed prices of the root's closed children, and of the paid ones.
    hired: u128,
    paid: u128,
}

impl World {
    fn new(funding: u128, bond: u128, fee: u128) -> Self {
        let mut ledger = Ledger::new(fee);
        ledger.fund(funding).unwrap();
        if bond > 0 {
            ledger.add_bond(bond).unwrap();
        }
        World {
            nodes: vec![Node {
                parent: None,
                real: ledger.ft_balance,
                ledger,
                status: Status::Open,
                closed: false,
                children_open: 0,
                returned: 0,
                note_sold: false,
                settle_started: false,
                total: funding,
            }],
            funding,
            bond,
            fee,
            lead_got: 0,
            hired: 0,
            paid: 0,
            paid_out: 0,
            worker_got: 0,
            treasury_spent: 0,
            treasury_got: 0,
            note_face: 0,
            note_fees: 0,
        }
    }

    fn pick(&self, i: usize) -> usize {
        i % self.nodes.len()
    }

    fn live(&self, i: usize) -> bool {
        !self.nodes[i].closed
    }

    fn apply(&mut self, op: &Op) {
        match *op {
            Op::Hire {
                parent,
                amount,
                sub,
                fail,
            } => {
                let p = self.pick(parent);
                if !self.live(p)
                    || self.nodes[p].status != Status::Open
                    || self.nodes[p].settle_started
                {
                    return;
                }
                let (amount, sub) = (amount as u128, sub as u128);
                let total = amount + sub;
                if self.nodes[p].ledger.allocate(total).is_err() {
                    return;
                }
                if fail {
                    // Create or funding failed: the callback rolls back.
                    self.nodes[p].ledger.release(total).unwrap();
                    return;
                }
                self.nodes[p].real -= total;
                let mut ledger = Ledger::new(amount);
                ledger.fund(total).unwrap();
                self.nodes[p].children_open += 1;
                self.nodes.push(Node {
                    parent: Some(p),
                    real: total,
                    ledger,
                    status: Status::Open,
                    closed: false,
                    children_open: 0,
                    returned: 0,
                    note_sold: false,
                    settle_started: false,
                    total,
                });
            }
            Op::Deliver(n) => {
                let n = self.pick(n);
                if n == 0 || !self.live(n) {
                    return;
                }
                if let Ok(s) = transition(self.nodes[n].status, Event::Deliver) {
                    self.nodes[n].status = s;
                }
            }
            Op::Check(n, pass) => {
                let n = self.pick(n);
                if !self.live(n) {
                    return;
                }
                let e = if pass {
                    Event::CheckPassed
                } else {
                    Event::CheckFailed
                };
                if let Ok(s) = transition(self.nodes[n].status, e) {
                    self.nodes[n].status = s;
                }
            }
            Op::Buy(n) => {
                let n = self.pick(n);
                let node = &mut self.nodes[n];
                if n == 0 || node.closed {
                    return;
                }
                // Receipt verification is assumed passed in the model.
                let Some(q) = node
                    .ledger
                    .sellable_quote(node.status, true, node.note_sold)
                else {
                    return;
                };
                // The lead's treasury pays the quote from outside the tree.
                node.note_sold = true;
                self.treasury_spent += q;
                self.worker_got += q;
                self.note_face += node.ledger.amount;
                self.note_fees += ht_core::money::note_fee(node.ledger.amount).unwrap();
            }
            Op::Settle {
                node,
                payee_ok,
                rest_ok,
                part,
            } => self.settle(self.pick(node), payee_ok, rest_ok, part),
        }
    }

    fn settle(&mut self, n: usize, payee_ok: bool, rest_ok: bool, part: Option<u16>) {
        if !self.live(n) || self.nodes[n].children_open != 0 {
            return;
        }
        let prev = self.nodes[n].status;
        let Ok(settling) = transition(prev, Event::StartSettle) else {
            return;
        };
        let pay_payee = matches!(prev, Status::Delivered | Status::Checked);
        self.nodes[n].settle_started = true;
        let split = if self.nodes[n].parent.is_none() {
            self.nodes[n]
                .ledger
                .settle_split_root(self.paid, self.hired)
                .unwrap()
        } else {
            self.nodes[n].ledger.settle_split(pay_payee).unwrap()
        };
        self.nodes[n].ledger.begin_payout(split).unwrap();
        self.nodes[n].status = settling;

        let payee_ok = payee_ok || split.to_payee == 0;
        let rest_ok = rest_ok || split.to_rest == 0;
        // What really left for the payee: all, nothing, or (a routed leg
        // whose receiver kept only part) a share of it.
        let used = match (payee_ok, part) {
            (false, _) => 0,
            (true, None) => split.to_payee,
            (true, Some(p)) => mul_div_floor(split.to_payee, p as u128, 1000).unwrap(),
        };
        if used > 0 {
            self.nodes[n].real -= used;
            self.paid_out += used;
            if self.nodes[n].parent.is_none() {
                self.lead_got += used;
            } else if self.nodes[n].note_sold {
                self.treasury_got += used;
            } else {
                self.worker_got += used;
            }
        }
        if rest_ok && split.to_rest > 0 {
            self.nodes[n].real -= split.to_rest;
            match self.nodes[n].parent {
                Some(p) => {
                    self.nodes[p].real += split.to_rest;
                    self.nodes[n].returned += split.to_rest;
                }
                None => self.paid_out += split.to_rest, // root: back to the client
            }
        }
        self.nodes[n]
            .ledger
            .payee_result(split.to_payee, used)
            .unwrap();
        self.nodes[n]
            .ledger
            .payout_result(
                Split {
                    to_payee: 0,
                    to_rest: split.to_rest,
                },
                true,
                rest_ok,
            )
            .unwrap();

        if self.nodes[n].ledger.ft_balance == 0 {
            self.nodes[n].status = transition(settling, Event::SettleOk).unwrap();
            assert_eq!(self.nodes[n].real, 0, "delete only with zero balance");
            self.nodes[n].closed = true;
            if let Some(p) = self.nodes[n].parent {
                let returned = self.nodes[n].returned;
                let allocation = self.nodes[n].ledger.funded;
                self.nodes[p].ledger.credit(returned, allocation).unwrap();
                self.nodes[p].children_open -= 1;
                if p == 0 {
                    // The child's notice: was its worker paid (at settle,
                    // or early through a note)?
                    let node = &self.nodes[n];
                    self.hired += node.total;
                    if node.note_sold || node.ledger.payee_paid {
                        self.paid += node.total;
                    }
                }
            }
        } else {
            self.nodes[n].status = transition(settling, Event::SettleFailed(prev)).unwrap();
        }
    }

    fn check_invariants(&self) {
        let tree: u128 = self.nodes.iter().map(|n| n.real).sum();
        assert_eq!(
            tree + self.paid_out,
            self.funding + self.bond,
            "money rule: tree {tree} + paid_out {} != funding {} + bond {}",
            self.paid_out,
            self.funding,
            self.bond
        );
        for (i, n) in self.nodes.iter().enumerate() {
            if n.closed {
                continue;
            }
            let pending: u128 = self
                .nodes
                .iter()
                .filter(|c| c.parent == Some(i) && !c.closed)
                .map(|c| c.returned)
                .sum();
            assert_eq!(
                n.ledger.ft_balance + pending,
                n.real,
                "ledger mirrors real balance at node {i}"
            );
            assert!(n.ledger.allocated <= n.ledger.funded);
        }
        // Payee payments out of the tree never exceed what left the tree.
        assert!(self.worker_got - self.treasury_spent + self.treasury_got <= self.paid_out);
    }

    /// Settle everything bottom-up with every transfer succeeding.
    fn drain(&mut self) {
        for _ in 0..self.nodes.len() + 1 {
            for i in (0..self.nodes.len()).rev() {
                self.settle(i, true, true, None);
            }
        }
    }
}

fn cases() -> u32 {
    std::env::var("PROP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    #[test]
    fn money_is_conserved_in_any_tree(
        funding in 1u64..=50_000_000,
        bond in 0u64..=5_000_000,
        fee_pct in 0u64..=40,
        ops in prop::collection::vec(op(), 0..60),
    ) {
        let fee = (funding * fee_pct / 100) as u128;
        let mut w = World::new(funding as u128, bond as u128, fee);
        w.check_invariants();
        for op in &ops {
            w.apply(op);
            w.check_invariants();
        }
        w.drain();
        w.check_invariants();
        // Everything closed: the whole budget plus bond has left the tree.
        prop_assert!(w.nodes.iter().all(|n| n.closed));
        prop_assert_eq!(w.paid_out, w.funding + w.bond);
        // Every bought note was paid to the treasury at settle; its net gain
        // is exactly the 1% fee on those notes.
        prop_assert_eq!(w.treasury_got, w.note_face);
        prop_assert_eq!(w.treasury_got - w.treasury_spent, w.note_fees);
        // The lead got its bond plus exactly the fee it earned: pro rata to
        // the root's paid parts over at least half the work budget, rounded
        // down, never more than the fee; no part paid, no fee.
        prop_assert!(w.paid <= w.hired);
        let floor = fee_floor(w.funding - w.fee).unwrap();
        let earned = earned_fee(w.fee, w.paid, w.hired, floor).unwrap();
        prop_assert!(earned <= w.fee);
        if w.hired < floor {
            prop_assert!(earned < w.fee || w.fee == 0, "under the floor: never the whole fee");
        }
        if w.paid == 0 {
            prop_assert_eq!(earned, 0);
        }
        prop_assert_eq!(w.lead_got, w.bond + earned);
    }

    #[test]
    fn earned_fee_is_exact_and_capped(
        fee in any::<u128>(),
        a in any::<u128>(),
        b in any::<u128>(),
        floor in prop_oneof![Just(0u128), any::<u128>()],
        small in any::<(u64, u64, u64, u64)>(),
    ) {
        let (paid, hired) = (a.min(b), a.max(b));
        let got = earned_fee(fee, paid, hired, floor).unwrap();
        prop_assert!(got <= fee);
        // Monotone in what was paid; everything paid earns the whole fee
        // once the lead hired at least the floor.
        prop_assert!(earned_fee(fee, paid / 2, hired, floor).unwrap() <= got);
        if hired > 0 && hired >= floor {
            prop_assert_eq!(earned_fee(fee, hired, hired, floor).unwrap(), fee);
        }
        // The floor is respected: under it the lead never earns the whole
        // fee, and a higher floor never pays more.
        if hired < floor && fee > 0 {
            prop_assert!(got < fee);
        }
        prop_assert!(earned_fee(fee, paid, hired, floor.saturating_add(floor / 2 + 1)).unwrap() <= got);
        if hired > 0 && floor <= hired {
            // Symmetric in fee and paid (both are factors of one product).
            prop_assert_eq!(mul_div_floor(paid, fee.min(hired), hired).unwrap(),
                earned_fee(fee.min(hired), paid, hired, floor).unwrap());
        }
        let (f, x, y, l) = (small.0 as u128, small.1 as u128, small.2 as u128, small.3 as u128);
        let (p, h) = (x.min(y), x.max(y));
        let want = if p == 0 { 0 } else { (f * p) / h.max(l) };
        prop_assert_eq!(earned_fee(f, p, h, l).unwrap(), want);
    }

    #[test]
    fn the_fee_floor_is_half_the_work_budget_rounded_up(budget in any::<u128>()) {
        let floor = fee_floor(budget).unwrap();
        prop_assert_eq!(floor, budget / 2 + budget % 2);
    }

    #[test]
    fn quote_never_exceeds_amount_and_fee_is_at_most_one_percent(amount in 1u128..=u128::MAX / 100) {
        let q = note_quote(amount).unwrap();
        prop_assert!(q <= amount);
        prop_assert!((amount - q) * 100 <= amount);
    }

    #[test]
    fn binding_parser_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = ht_core::binding::parse_tags(&bytes);
        let _ = ht_core::binding::check_binding(&bytes, "t1.job", &[0; 32], 1);
    }

    #[test]
    fn rendered_tag_always_parses_back(
        task in "[a-z0-9]{1,10}(\\.[a-z0-9]{1,10}){0,3}",
        spec in any::<[u8; 32]>(),
        seq in any::<u32>(),
        pre in "[ a-zA-Z,:]{0,20}",
        post in "[ ,.\"}]{0,20}",
    ) {
        prop_assume!(task.len() >= 2);
        let tag = ht_core::binding::BindingTag { task: task.clone(), spec_hash: spec, seq };
        let req = format!("{pre}{}{post}", tag.render());
        prop_assert_eq!(ht_core::binding::check_binding(req.as_bytes(), &task, &spec, seq), Ok(()));
    }

    #[test]
    fn transition_never_leaves_a_terminal_status(from in 0usize..7, ev in 0usize..9, prev in 0usize..7) {
        let from = Status::ALL[from];
        let events = [
            Event::Deliver, Event::CheckPassed, Event::CheckFailed, Event::Dispute,
            Event::DisputeUpheld, Event::DisputeRejected, Event::StartSettle,
            Event::SettleOk, Event::SettleFailed(Status::ALL[prev]),
        ];
        if let Ok(to) = transition(from, events[ev]) {
            prop_assert_ne!(from, Status::Settled);
            if to == Status::Settled { prop_assert_eq!(from, Status::Settling); }
        }
    }
}
