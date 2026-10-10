use super::*;
use craftworks_reputation::statement::Line;
use std::collections::BTreeMap;

const WREN: Tok = Tok { definition: [7; 32], item: [0; 32] };

fn d(n: u8) -> Did {
    [n; 32]
}

/// A statement tree: DID → line (only the allowance matters here), its root, and a proof of each line.
fn statement(lines: &[(Did, u128)]) -> ([u8; 32], BTreeMap<[u8; 32], Vec<u8>>) {
    let mut blocks = BTreeMap::new();
    let (root, _) = engine::repair::build_tree(lines.iter().map(|(d, a)| (d.to_vec(), Line { rep: 1, keeper: 0, allowance: *a, witness: 0 }.encode())), &mut blocks).unwrap();
    (root, blocks)
}
fn proof(blocks: &BTreeMap<[u8; 32], Vec<u8>>, root: &[u8; 32], did: &Did) -> Vec<u8> {
    freenet_prolly::proof::prove(blocks, root, did).unwrap().encode()
}

#[test]
fn the_verdict_pays_each_line_and_refunds_the_rest() {
    let p = Params { token: WREN, epoch: 5 };
    assert_eq!(Params::decode(&p.encode()), Some(p));
    let (root, blocks) = statement(&[(d(1), 1 * UNIT), (d(2), UNIT / 2), (d(3), 0)]);
    let lines: Vec<(Did, Vec<u8>)> = [d(1), d(2), d(3)].iter().map(|x| (*x, proof(&blocks, &root, x))).collect();
    let treasury: EscrowRef = (WREN, d(9), [1; 16]);
    let v = verdict(&p, &[(treasury, POT)], &root, &lines).unwrap();
    // 1 and 2 paid their lines; 3's zero is no payout; the rest of the pot back to the treasury.
    assert_eq!(v.section.pays.iter().map(|x| (x.2, x.3)).collect::<Vec<_>>(), vec![(d(1), UNIT), (d(2), UNIT / 2)]);
    assert_eq!(v.section.refunds, vec![(treasury, POT - UNIT - UNIT / 2)]);
    assert_eq!(check(&p, &v), Ok(()));
    assert_eq!(State::decode(&v.encode()), Some(v.clone()));
    // Empty: valid, not decided; it takes the verdict; a verdict held is never replaced.
    let empty = State::default();
    assert_eq!((check(&p, &empty), empty.decided()), (Ok(()), false));
    assert_eq!(State::decode(&empty.encode()), Some(empty.clone()));
    assert_eq!(merge(&p, &empty, &v), v);
    let (root2, blocks2) = statement(&[(d(1), 2 * UNIT)]);
    let other = verdict(&p, &[(treasury, POT)], &root2, &[(d(1), proof(&blocks2, &root2, &d(1)))]).unwrap();
    assert_eq!(check(&p, &other), Ok(()));
    assert_eq!(merge(&p, &v, &other), v);
    // More owed than escrowed: no verdict.
    assert_eq!(verdict(&p, &[(treasury, UNIT)], &root, &lines).err(), Some("more owed than escrowed"));
}

#[test]
fn a_verdict_that_does_not_follow_its_lines_is_refused() {
    let p = Params { token: WREN, epoch: 5 };
    let (root, blocks) = statement(&[(d(1), UNIT), (d(2), UNIT)]);
    let lines: Vec<(Did, Vec<u8>)> = [d(1), d(2)].iter().map(|x| (*x, proof(&blocks, &root, x))).collect();
    let treasury: EscrowRef = (WREN, d(9), [1; 16]);
    let v = verdict(&p, &[(treasury, POT)], &root, &lines).unwrap();
    let bad = |f: &dyn Fn(&mut State)| {
        let mut s = v.clone();
        f(&mut s);
        check(&p, &s)
    };
    // Paid more than its line; to someone else; under another key; in another token; a proof dropped.
    // (More than its line, the totals still balanced — taken from the refund: only the line check can refuse it.)
    assert_eq!(
        bad(&|s| {
            s.section.pays[0].3 += 1;
            s.section.refunds[0].1 -= 1;
        }),
        Err("a payout that is not its line's allowance")
    );
    assert_eq!(bad(&|s| s.section.pays[0].3 += 1), Err("a program paying out more than was escrowed with it"));
    assert_eq!(bad(&|s| s.section.pays[0].2 = d(3)), Err("a payout without its proof"));
    assert_eq!(bad(&|s| s.section.pays[0].0 = pay_key(6, &d(1))), Err("a payout that is not its account's"));
    assert_eq!(bad(&|s| s.proofs.pop().map(|_| ()).unwrap()), Err("a payout without its proof"));
    // A proof under another root: refused.
    assert_eq!(bad(&|s| s.root = [3; 32]), Err("a payout that is not its line's allowance"));
    // The rest kept back (not refunded), or an escrow of another token or terms.
    assert_eq!(bad(&|s| s.section.refunds[0].1 -= 1), Err("a verdict that does not settle every escrow"));
    assert_eq!(bad(&|s| s.section.escrows[0].2 = [0; 32]), Err("an escrow that is not this program's"));
    let other: EscrowRef = (Tok { definition: [8; 32], item: [0; 32] }, d(9), [1; 16]);
    assert!(verdict(&p, &[(other, POT)], &root, &lines).map(|s| check(&p, &s)).unwrap().is_err());
    // An undecided state listing an escrow: refused (escrows are listed with the verdict).
    assert_eq!(check(&p, &State { section: Section { escrows: v.section.escrows.clone(), ..Default::default() }, ..Default::default() }), Err("an undecided state that lists something"));
}
