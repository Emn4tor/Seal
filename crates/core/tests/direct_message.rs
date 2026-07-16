use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use identity::Identity;
use p2p_core::{AttachmentPayload, ChatEvent, ChatNode};

/// Full 1:1 round trip over the real libp2p transport: Alice establishes an
/// Olm session using a one-time key Bob published, sends an encrypted
/// message, Bob decrypts and replies, and Alice decrypts the reply using
/// the now bidirectionally-established session. No shortcuts — real
/// transport, real Olm crypto, both directions.
#[tokio::test]
async fn full_1to1_round_trip_over_real_transport() {
    let mut alice = ChatNode::new(Identity::generate()).expect("build alice");
    let mut bob = ChatNode::new(Identity::generate()).expect("build bob");

    let alice_user_id = alice.identity.user_id();
    let bob_user_id = bob.identity.user_id();
    let alice_curve = alice.identity.curve25519_public_base64();
    let bob_curve = bob.identity.curve25519_public_base64();
    let alice_peer_id = alice.local_peer_id();
    let bob_peer_id = bob.local_peer_id();

    // Bob publishes a one-time key, as he would to the directory server.
    bob.identity.account_mut().generate_one_time_keys(1);
    let bob_otk = *bob
        .identity
        .account()
        .one_time_keys()
        .values()
        .next()
        .unwrap();
    let bob_otk_b64 = STANDARD.encode(bob_otk.as_bytes());

    alice.add_contact(&bob_user_id, &bob_curve, bob_peer_id, vec![]);
    bob.add_contact(&alice_user_id, &alice_curve, alice_peer_id, vec![]);

    alice
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .expect("alice listens");
    let alice_addr = alice.wait_for_listen_addr().await;
    bob.dial(alice_addr).expect("bob dials alice");

    alice
        .ensure_outbound_session(&bob_user_id, &bob_otk_b64)
        .expect("alice establishes outbound olm session");

    // Alice's first message also carries a small attachment — this is the
    // round trip that matters for attachments: Olm encryption/decryption is
    // payload-agnostic (see crypto-session/src/envelope.rs), so this proves
    // the bytes survive real transport + real Olm crypto intact, not just a
    // serialize/deserialize round trip in isolation.
    let attachment = AttachmentPayload {
        filename: "note.txt".to_string(),
        mime_type: "text/plain".to_string(),
        exif_stripped: false,
        data: b"not a real file, just some bytes".to_vec(),
    };

    let mut alice_connected = false;
    let mut bob_connected = false;
    let mut sent_initial_message = false;
    let mut bob_got_message = false;
    let mut alice_got_reply = false;

    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if bob_got_message && alice_got_reply {
                break;
            }
            tokio::select! {
                event = alice.next_event() => {
                    match event {
                        ChatEvent::Connected(_) => alice_connected = true,
                        ChatEvent::DirectMessage { from, body, attachment } => {
                            assert_eq!(from, bob_user_id);
                            assert_eq!(body, "hi alice");
                            assert!(attachment.is_none());
                            alice_got_reply = true;
                        }
                        _ => {}
                    }
                }
                event = bob.next_event() => {
                    match event {
                        ChatEvent::Connected(_) => bob_connected = true,
                        ChatEvent::DirectMessage { from, body, attachment } => {
                            assert_eq!(from, alice_user_id);
                            assert_eq!(body, "hello bob");
                            let attachment = attachment.expect("alice's message had an attachment");
                            assert_eq!(attachment.filename, "note.txt");
                            assert_eq!(attachment.mime_type, "text/plain");
                            assert!(!attachment.exif_stripped);
                            assert_eq!(attachment.data, b"not a real file, just some bytes");
                            bob_got_message = true;
                            bob.send_direct_message(&alice_user_id, "hi alice", None)
                                .expect("bob replies");
                        }
                        _ => {}
                    }
                }
            }

            if alice_connected && bob_connected && !sent_initial_message {
                sent_initial_message = true;
                alice
                    .send_direct_message(&bob_user_id, "hello bob", Some(attachment.clone()))
                    .expect("alice sends first message");
            }
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "timed out: bob_got_message={bob_got_message}, alice_got_reply={alice_got_reply}"
    );
}
