use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use identity::Identity;
use p2p_core::{AttachmentPayload, ChatEvent, ChatNode};

/// 3-node group round trip over the real libp2p transport: the group owner
/// creates a Megolm outbound session, shares it 1:1-Olm-encrypted with two
/// members, then publishes a group message over gossipsub that both members
/// decrypt. This is the "no chat control, no server-side trace" path in
/// full: the key share never touches any server, and the group message body
/// is only ever seen by the sender and by peers holding the shared key.
#[tokio::test]
async fn three_node_group_round_trip_including_key_share() {
    let mut owner = ChatNode::new(Identity::generate()).expect("build owner");
    let mut member_b = ChatNode::new(Identity::generate()).expect("build member b");
    let mut member_c = ChatNode::new(Identity::generate()).expect("build member c");

    let owner_id = owner.identity.user_id();
    let b_id = member_b.identity.user_id();
    let c_id = member_c.identity.user_id();

    let owner_curve = owner.identity.curve25519_public_base64();
    let b_curve = member_b.identity.curve25519_public_base64();
    let c_curve = member_c.identity.curve25519_public_base64();

    let owner_peer = owner.local_peer_id();
    let b_peer = member_b.local_peer_id();
    let c_peer = member_c.local_peer_id();

    // One-time keys B and C would have published to the directory server,
    // needed for the owner to establish 1:1 Olm sessions to key-share with
    // them.
    member_b.identity.account_mut().generate_one_time_keys(1);
    let b_otk = *member_b
        .identity
        .account()
        .one_time_keys()
        .values()
        .next()
        .unwrap();
    let b_otk_b64 = STANDARD.encode(b_otk.as_bytes());

    member_c.identity.account_mut().generate_one_time_keys(1);
    let c_otk = *member_c
        .identity
        .account()
        .one_time_keys()
        .values()
        .next()
        .unwrap();
    let c_otk_b64 = STANDARD.encode(c_otk.as_bytes());

    // Full triangle of contacts/connections so gossipsub has every edge
    // available for mesh formation with such a small peer set.
    owner.add_contact(&b_id, &b_curve, b_peer, vec![]);
    owner.add_contact(&c_id, &c_curve, c_peer, vec![]);
    member_b.add_contact(&owner_id, &owner_curve, owner_peer, vec![]);
    member_b.add_contact(&c_id, &c_curve, c_peer, vec![]);
    member_c.add_contact(&owner_id, &owner_curve, owner_peer, vec![]);
    member_c.add_contact(&b_id, &b_curve, b_peer, vec![]);

    owner
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .expect("owner listens");
    let owner_addr = owner.wait_for_listen_addr().await;

    member_b
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .expect("member_b listens");
    let b_addr = member_b.wait_for_listen_addr().await;

    member_b.dial(owner_addr.clone()).expect("b dials owner");
    member_c.dial(owner_addr).expect("c dials owner");
    member_c.dial(b_addr).expect("c dials b");

    owner
        .ensure_outbound_session(&b_id, &b_otk_b64)
        .expect("owner establishes olm session with b");
    owner
        .ensure_outbound_session(&c_id, &c_otk_b64)
        .expect("owner establishes olm session with c");

    const GROUP_ID: &str = "group-1";
    const CHANNEL_ID: &str = "channel-1";
    owner.create_group(GROUP_ID);
    member_b.join_group_topic(GROUP_ID);
    member_c.join_group_topic(GROUP_ID);

    let mut b_subscribed = false;
    let mut c_subscribed = false;
    let mut shared_keys = false;
    let mut b_got_key = false;
    let mut c_got_key = false;
    let mut sent_group_message = false;
    let mut b_got_message = false;
    let mut c_got_message = false;

    let result = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if b_got_message && c_got_message {
                break;
            }
            tokio::select! {
                event = owner.next_event() => {
                    if let ChatEvent::GossipSubscribed { peer_id, .. } = event {
                        if peer_id == b_peer { b_subscribed = true; }
                        if peer_id == c_peer { c_subscribed = true; }
                    }
                }
                event = member_b.next_event() => {
                    if let ChatEvent::GroupKeyReceived { from, .. } = &event
                        && *from == owner_id
                    {
                        b_got_key = true;
                    }
                    if let ChatEvent::GroupMessage { from, channel_id, body, attachment, .. } = event {
                        assert_eq!(from, owner_id);
                        assert_eq!(channel_id, CHANNEL_ID);
                        assert_eq!(body, "hello group");
                        let attachment = attachment.expect("owner's group message had an attachment");
                        assert_eq!(attachment.filename, "photo.jpg");
                        assert_eq!(attachment.data, b"pretend-image-bytes");
                        b_got_message = true;
                    }
                }
                event = member_c.next_event() => {
                    if let ChatEvent::GroupKeyReceived { from, .. } = &event
                        && *from == owner_id
                    {
                        c_got_key = true;
                    }
                    if let ChatEvent::GroupMessage { from, channel_id, body, attachment, .. } = event {
                        assert_eq!(from, owner_id);
                        assert_eq!(channel_id, CHANNEL_ID);
                        assert_eq!(body, "hello group");
                        let attachment = attachment.expect("owner's group message had an attachment");
                        assert_eq!(attachment.filename, "photo.jpg");
                        assert_eq!(attachment.data, b"pretend-image-bytes");
                        c_got_message = true;
                    }
                }
            }

            if b_subscribed && c_subscribed && !shared_keys {
                shared_keys = true;
                owner
                    .share_group_key(GROUP_ID, &b_id)
                    .expect("owner shares key with b");
                owner
                    .share_group_key(GROUP_ID, &c_id)
                    .expect("owner shares key with c");
            }

            if b_got_key && c_got_key && !sent_group_message {
                sent_group_message = true;
                let attachment = AttachmentPayload {
                    filename: "photo.jpg".to_string(),
                    mime_type: "image/jpeg".to_string(),
                    exif_stripped: true,
                    data: b"pretend-image-bytes".to_vec(),
                };
                owner
                    .send_group_message(GROUP_ID, CHANNEL_ID, "hello group", Some(attachment))
                    .expect("owner sends group message");
            }
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "timed out: b_subscribed={b_subscribed} c_subscribed={c_subscribed} \
         b_got_key={b_got_key} c_got_key={c_got_key} \
         b_got_message={b_got_message} c_got_message={c_got_message}"
    );
}
